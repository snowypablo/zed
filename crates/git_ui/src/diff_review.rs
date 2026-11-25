use db::kvp::KeyValueStore;
use editor::actions::RunDiffReview;
use gpui::{Anchor, App, ClickEvent, FocusHandle, Global, TaskExt as _, Window};
use ui::{ContextMenu, PopoverMenu, Tooltip, prelude::*};
use util::ResultExt as _;

const REVIEW_AGENT_KEY: &str = "git.diff_review.selected_agent";

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ReviewAgent {
    Claude,
    Codex,
}

struct SelectedReviewAgent(Option<ReviewAgent>);

impl Global for SelectedReviewAgent {}

impl ReviewAgent {
    const ALL: [Self; 2] = [Self::Claude, Self::Codex];

    pub fn label(self) -> &'static str {
        match self {
            Self::Claude => "Claude",
            Self::Codex => "Codex",
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            Self::Claude => "claude-acp",
            Self::Codex => "codex-acp",
        }
    }

    pub fn selected(cx: &mut App) -> Option<Self> {
        if let Some(selected) = cx.try_global::<SelectedReviewAgent>() {
            return selected.0;
        }
        let selected = KeyValueStore::global(cx)
            .read_kvp(REVIEW_AGENT_KEY)
            .log_err()
            .flatten()
            .and_then(|id| Self::ALL.into_iter().find(|agent| agent.id() == id));
        cx.set_global(SelectedReviewAgent(selected));
        selected
    }

    fn select(self, cx: &mut App) {
        cx.set_global(SelectedReviewAgent(Some(self)));
        cx.refresh_windows();
        let store = KeyValueStore::global(cx);
        cx.spawn(async move |cx| {
            if cx.update(Self::selected) == Some(self) {
                store
                    .write_kvp(REVIEW_AGENT_KEY.into(), self.id().into())
                    .await?;
            }
            anyhow::Ok(())
        })
        .detach_and_log_err(cx);
    }
}

pub(crate) fn render_run_review_controls(
    review_count: usize,
    focus_handle: &FocusHandle,
    on_run: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    cx: &mut App,
) -> impl IntoElement {
    let selected = ReviewAgent::selected(cx);
    h_flex()
        .gap_0p5()
        .child(
            PopoverMenu::new("diff-review-agent-picker")
                .anchor(Anchor::TopRight)
                .attach(Anchor::BottomRight)
                .menu(move |window, cx| {
                    Some(ContextMenu::build(window, cx, |mut menu, _, _| {
                        for agent in ReviewAgent::ALL {
                            menu = menu.toggleable_entry(
                                agent.label(),
                                Some(agent) == selected,
                                IconPosition::Start,
                                None,
                                move |_, cx| agent.select(cx),
                            );
                        }
                        menu
                    }))
                })
                .trigger_with_tooltip(
                    Button::new(
                        "diff-review-agent",
                        selected.map_or("Choose agent", ReviewAgent::label),
                    )
                    .aria_label(selected.map_or_else(
                        || "Choose review agent".to_string(),
                        |agent| format!("Review agent: {}", agent.label()),
                    ))
                    .end_icon(
                        Icon::new(IconName::ChevronDown)
                            .size(IconSize::XSmall)
                            .color(Color::Muted),
                    ),
                    Tooltip::text("Select review agent"),
                ),
        )
        .child(
            Button::new("send-review", format!("Run Review ({review_count})"))
                .start_icon(
                    Icon::new(IconName::ZedAssistant)
                        .size(IconSize::Small)
                        .color(Color::Muted),
                )
                .disabled(selected.is_none())
                .tooltip(Tooltip::for_action_title_in(
                    "Start an agent to address draft review comments",
                    &RunDiffReview,
                    focus_handle,
                ))
                .on_click(on_run),
        )
}
