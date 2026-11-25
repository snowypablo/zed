use agent_ui::AgentPanel;
use anyhow::{Context as _, Result, ensure};
use collections::HashMap;
use editor::{
    Editor,
    actions::{ReplyToDiffReview, RunDiffReview},
};
use git_ui::ReviewAgent;
use gpui::{Context, TaskExt as _, Window};
use serde::Deserialize;
use workspace::{Toast, Workspace, notifications::NotificationId};

pub(super) fn run_diff_review(
    workspace: &mut Workspace,
    _: &RunDiffReview,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    let Some(editor) = workspace
        .active_item(cx)
        .and_then(|item| item.act_as::<Editor>(cx))
    else {
        return;
    };
    let feedback = editor.read(cx).review_feedback(cx);
    if feedback.is_empty() {
        workspace.show_error("There are no review drafts ready to run.", cx);
        return;
    }
    let Some(agent) = ReviewAgent::selected(cx) else {
        workspace.show_error("Choose a review agent in the diff toolbar first.", cx);
        return;
    };
    let agent_name = agent.label();
    let comment_ids: Vec<_> = feedback.iter().map(|(id, _)| *id).collect();
    let start = (|| -> Result<_> {
        let worktree = editor
            .read(cx)
            .review_worktree(&comment_ids, cx)
            .context("The review comments must belong to one worktree.")?;
        let feedback = feedback
            .into_iter()
            .map(|(id, item)| serde_json::json!({"id": id, "feedback": item}))
            .collect::<Vec<_>>();
        let feedback = serde_json::to_string_pretty(&feedback)?;
        let prompt = format!(
            "Address this human-authored review feedback in the current worktree. Read the code and use the file, line range, and excerpt to locate each item. Make the requested fixes where appropriate. Answer questions without editing code when the reviewer only asks for an explanation. Keep replies concise. Preserve existing unrelated changes and leave the edits uncommitted. End your final assistant message with a fenced json code block containing an object with a replies array. Each reply must have the original integer id and a body explaining what changed or what remains. Include at most one reply per id. Omit items that need no reply.\n\n{feedback}"
        );
        let panel = workspace
            .panel::<AgentPanel>(cx)
            .context("The Agent Panel is unavailable.")?;
        panel.update(cx, |panel, cx| {
            panel.run_diff_review(agent.id().into(), &worktree, prompt, window, cx)
        })
    })();
    let (thread_id, run) = match start {
        Ok(run) => run,
        Err(error) => {
            workspace.show_error(error.to_string(), cx);
            return;
        }
    };

    editor.update(cx, |editor, cx| editor.begin_diff_review(&comment_ids, cx));
    workspace.show_toast(
        Toast::new(
            NotificationId::unique::<RunDiffReview>(),
            format!("{agent_name} is addressing your review"),
        ),
        cx,
    );

    let editor = editor.downgrade();
    cx.spawn_in(window, async move |workspace, cx| {
        let result = run
            .await
            .context("The review session ended before returning replies.")
            .and_then(|result| result)
            .and_then(|response| parse_replies(&response, &comment_ids));

        workspace.update_in(cx, |workspace, _, cx| {
            if let Some(editor) = editor.upgrade() {
                editor.update(cx, |editor, cx| {
                    editor.finish_diff_review(
                        &comment_ids,
                        thread_id.into(),
                        agent_name.into(),
                        result.as_ref().ok(),
                        cx,
                    );
                });
            }
            match result {
                Ok(replies) => {
                    let count = replies.len();
                    let noun = if count == 1 { "reply" } else { "replies" };
                    workspace.show_toast(
                        Toast::new(
                            NotificationId::unique::<RunDiffReview>(),
                            format!("{agent_name} finished the review · {count} {noun}"),
                        ),
                        cx,
                    );
                }
                Err(error) => workspace.show_error(error.to_string(), cx),
            }
        })
    })
    .detach_and_log_err(cx);
}

pub(super) fn reply_to_diff_review(
    workspace: &mut Workspace,
    action: &ReplyToDiffReview,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    let Some(editor) = workspace
        .active_item(cx)
        .and_then(|item| item.act_as::<Editor>(cx))
    else {
        return;
    };
    let comment_id = action.id;
    let Some((thread_id, body)) = editor.read(cx).diff_review_reply(comment_id, cx) else {
        return;
    };
    let prompt = format!(
        "Continue the inline review discussion for the original comment with ID {comment_id}. Use our conversation about that comment for context. If the reviewer asks only for an explanation, leave the code alone. Leave any requested edits uncommitted. End your final assistant message with a fenced json code block containing an object with a replies array. Include exactly one reply with the integer id {comment_id} and a body string containing only the concise response to post inline. Keep progress updates, working notes, and commentary outside the body.

{body}"
    );
    let run = workspace
        .panel::<AgentPanel>(cx)
        .context("The Agent Panel is unavailable.")
        .and_then(|panel| {
            panel.update(cx, |panel, cx| {
                panel.continue_diff_review(thread_id.into(), prompt, window, cx)
            })
        });
    let run = match run {
        Ok(run) => run,
        Err(error) => {
            workspace.show_error(error.to_string(), cx);
            return;
        }
    };
    editor.update(cx, |editor, cx| {
        editor.begin_diff_review_reply(comment_id, body, window, cx);
    });

    let editor = editor.downgrade();
    cx.spawn_in(window, async move |workspace, cx| {
        let result = run
            .await
            .context("The review session ended before returning a reply.")
            .and_then(|result| result)
            .and_then(|response| parse_follow_up(&response, comment_id));
        workspace.update_in(cx, |workspace, window, cx| {
            let (reply, error) = match result {
                Ok(reply) => (Some(reply), None),
                Err(error) => (None, Some(error)),
            };
            if let Some(editor) = editor.upgrade() {
                editor.update(cx, |editor, cx| {
                    editor.finish_diff_review_reply(comment_id, thread_id, reply, window, cx);
                });
            }
            if let Some(error) = error {
                workspace.show_error(error.to_string(), cx);
            }
        })
    })
    .detach_and_log_err(cx);
}

#[derive(Deserialize)]
struct ReviewResponse {
    replies: Vec<ReviewReply>,
}

#[derive(Deserialize)]
struct ReviewReply {
    id: usize,
    body: String,
}

fn parse_replies(response: &str, comment_ids: &[usize]) -> Result<HashMap<usize, String>> {
    let response = response.trim();
    let response = response
        .rsplit_once("```json\n")
        .and_then(|(_, text)| text.strip_suffix("```"))
        .unwrap_or(response)
        .trim();
    let response: ReviewResponse = serde_json::from_str(response)
        .context("The agent did not return inline replies in the expected format. See the review thread for details.")?;
    let mut replies = HashMap::default();
    for reply in response.replies {
        ensure!(
            comment_ids.contains(&reply.id),
            "The agent returned a reply for an unknown comment. See the review thread for details."
        );
        let body = reply.body.trim();
        ensure!(
            !body.is_empty(),
            "The agent returned an empty review reply. See the review thread for details."
        );
        ensure!(
            replies.insert(reply.id, body.to_owned()).is_none(),
            "The agent returned duplicate replies. See the review thread for details."
        );
    }
    Ok(replies)
}

fn parse_follow_up(response: &str, comment_id: usize) -> Result<String> {
    parse_replies(response, &[comment_id])?
        .remove(&comment_id)
        .context("The review agent did not include a reply. See the review thread for details.")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn review_follow_up_posts_only_the_reply_body() {
        let response = concat!(
            "I'll inspect the parser, then post what changed.\n\n",
            "```json\n",
            r#"{"replies":[{"id":2,"body":"Handled empty input.\nThe tests passed."}]}"#,
            "\n```",
        );
        assert_eq!(
            parse_follow_up(response, 2).unwrap(),
            "Handled empty input.\nThe tests passed."
        );
        assert!(parse_follow_up(response, 4).is_err());
        assert!(parse_follow_up("I'll inspect the parser.", 2).is_err());
        assert!(parse_follow_up(r#"{"replies":[]}"#, 2).is_err());
    }

    #[test]
    fn review_replies_accept_only_the_submitted_ids() {
        let replies = parse_replies(
            r#"{"replies":[{"id":2,"body":" Changed the parser. "}]}"#,
            &[2, 4],
        )
        .unwrap();
        assert_eq!(
            replies.get(&2).map(String::as_str),
            Some("Changed the parser.")
        );
        assert!(
            parse_replies("Finished.\n```json\n{\"replies\":[]}\n```", &[2])
                .unwrap()
                .is_empty()
        );
        assert!(parse_replies(r#"{"replies":[{"id":8,"body":"Other"}]}"#, &[2]).is_err());
        assert!(
            parse_replies(
                r#"{"replies":[{"id":2,"body":"First"},{"id":2,"body":"Second"}]}"#,
                &[2]
            )
            .is_err()
        );
    }
}
