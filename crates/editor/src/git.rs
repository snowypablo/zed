pub(super) mod blame;

use super::*;
use ::git::{
    Oid, Restore, blame::BlameEntry, commit::ParsedCommitMessage, repository::RepoPath,
    status::FileStatus,
};
use buffer_diff::{BufferDiff, DiffHunkStatus, DiffHunkStatusKind};
use project::git_store::Repository;

#[derive(Clone)]
struct ResolvedDiffHunk {
    buffer_range: Range<text::Anchor>,
    diff_base_byte_range: Range<usize>,
    status: DiffHunkStatus,
}

#[derive(Clone)]
struct ResolvedDiffHunks {
    diff: Entity<BufferDiff>,
    buffer: Option<Entity<Buffer>>,
    hunks: Vec<ResolvedDiffHunk>,
}

pub trait DiffHunkRenderer {
    fn render_hunk_controls(
        &self,
        row: u32,
        status: &DiffHunkStatus,
        hunk_range: Range<Anchor>,
        is_created_file: bool,
        line_height: Pixels,
        editor: &Entity<Editor>,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyElement;

    fn render_hunk_as_staged(&self, status: &DiffHunkStatus, _cx: &App) -> bool {
        !status.has_secondary_hunk()
    }
}

pub struct DefaultDiffHunkRenderer;

impl DiffHunkRenderer for DefaultDiffHunkRenderer {
    fn render_hunk_controls(
        &self,
        row: u32,
        status: &DiffHunkStatus,
        hunk_range: Range<Anchor>,
        is_created_file: bool,
        line_height: Pixels,
        editor: &Entity<Editor>,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        render_diff_hunk_controls(
            row,
            status,
            hunk_range,
            is_created_file,
            line_height,
            editor,
            window,
            cx,
        )
    }
}

pub struct HiddenDiffHunkRenderer;

impl DiffHunkRenderer for HiddenDiffHunkRenderer {
    fn render_hunk_controls(
        &self,
        _row: u32,
        _status: &DiffHunkStatus,
        _hunk_range: Range<Anchor>,
        _is_created_file: bool,
        _line_height: Pixels,
        _editor: &Entity<Editor>,
        _window: &mut Window,
        _cx: &mut App,
    ) -> AnyElement {
        gpui::Empty.into_any_element()
    }
}

pub struct HiddenUnstagedDiffHunkRenderer;

impl DiffHunkRenderer for HiddenUnstagedDiffHunkRenderer {
    fn render_hunk_controls(
        &self,
        _row: u32,
        _status: &DiffHunkStatus,
        _hunk_range: Range<Anchor>,
        _is_created_file: bool,
        _line_height: Pixels,
        _editor: &Entity<Editor>,
        _window: &mut Window,
        _cx: &mut App,
    ) -> AnyElement {
        gpui::Empty.into_any_element()
    }

    fn render_hunk_as_staged(&self, _status: &DiffHunkStatus, _cx: &App) -> bool {
        false
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum DisplayDiffHunk {
    Folded {
        display_row: DisplayRow,
    },
    Unfolded {
        is_created_file: bool,
        diff_base_byte_range: Range<usize>,
        display_row_range: Range<DisplayRow>,
        multi_buffer_range: Range<Anchor>,
        status: DiffHunkStatus,
        word_diffs: Vec<Range<MultiBufferOffset>>,
    },
}

#[derive(Clone)]
pub(super) struct InlineBlamePopoverState {
    pub(super) scroll_handle: ScrollHandle,
    pub(super) commit_message: Option<ParsedCommitMessage>,
    pub(super) markdown: Entity<Markdown>,
}

pub(super) struct InlineBlamePopover {
    pub(super) position: gpui::Point<Pixels>,
    pub(super) hide_task: Option<Task<()>>,
    pub(super) popover_bounds: Option<Bounds<Pixels>>,
    pub(super) popover_state: InlineBlamePopoverState,
    pub(super) keyboard_grace: bool,
}

/// Represents a diff review button indicator that shows up when hovering over lines in the gutter
/// in diff view mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct PhantomDiffReviewIndicator {
    /// The starting anchor of the selection (or the only row if not dragging).
    pub(super) start: Anchor,
    /// The ending anchor of the selection. Equal to start_anchor for single-line selection.
    pub(super) end: Anchor,
    /// There's a small debounce between hovering over the line and showing the indicator.
    /// We don't want to show the indicator when moving the mouse from editor to e.g. project panel.
    pub(super) is_active: bool,
}

#[derive(Clone, Debug)]
pub(super) struct DiffReviewDragState {
    start_anchor: Anchor,
    current_anchor: Anchor,
}

/// Identifies a specific hunk in the diff buffer.
/// Used as a key to group comments by their location.
#[derive(Clone, Debug)]
pub(super) struct DiffHunkKey {
    /// The file path (relative to worktree) this hunk belongs to.
    pub(super) file_path: Arc<util::rel_path::RelPath>,
    /// An anchor at the start of the hunk. This tracks position as the buffer changes.
    pub(super) hunk_start_anchor: Anchor,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewCommentStatus {
    #[default]
    Draft,
    Queued,
    Sent,
    InReview,
    Outdated,
    Resolved,
}

impl ReviewCommentStatus {
    pub fn label(self) -> &'static str {
        match self {
            Self::Draft => "Draft",
            Self::Queued => "Queued",
            Self::Sent => "Sent",
            Self::InReview => "In review",
            Self::Outdated => "Outdated",
            Self::Resolved => "Resolved",
        }
    }

    pub fn is_sendable(self) -> bool {
        matches!(self, Self::Draft | Self::Queued)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReviewSessionSource {
    LocalDiff,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReviewComment {
    pub id: String,
    pub author: String,
    pub body: String,
    pub is_draft: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReviewThread {
    pub id: String,
    pub file_path: String,
    pub start_line: u32,
    pub end_line: u32,
    pub excerpt: String,
    pub status: ReviewCommentStatus,
    pub comments: Vec<ReviewComment>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReviewSession {
    pub source: ReviewSessionSource,
    pub threads: Vec<ReviewThread>,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct ReviewFeedback {
    pub worktree_name: Option<String>,
    pub file_path: String,
    pub start_line: u32,
    pub end_line: u32,
    pub excerpt: String,
    pub comment: String,
    #[serde(default)]
    pub status: ReviewCommentStatus,
}

/// A review comment stored locally before being sent to the Agent panel.
#[derive(Clone)]
pub(super) struct StoredReviewComment {
    /// Unique identifier for this comment (for edit/delete operations).
    pub(super) id: usize,
    /// The comment text entered by the user.
    pub(super) comment: String,
    /// Anchors for the code range being reviewed.
    pub(super) range: Range<Anchor>,
    excerpt: Arc<str>,
    pub(super) agent_reply: Option<(SharedString, String)>,
    /// Whether this comment is currently being edited inline.
    pub(super) is_editing: bool,
    /// Lifecycle status for the local draft thread.
    pub(super) status: ReviewCommentStatus,
}

pub(super) struct DiffReviewOverlay {
    pub(super) anchor_range: Range<Anchor>,
    pub(super) block_id: CustomBlockId,
    prompt_editor: Entity<Editor>,
    hunk_key: DiffHunkKey,
    pub(super) comment_id: Option<usize>,
    pub(super) comments_expanded: bool,
    inline_edit: Option<(Entity<Editor>, Subscription)>,
    user_avatar_uri: Option<SharedUri>,
    _subscription: Subscription,
}

impl DiffReviewDragState {
    pub(super) fn row_range(
        &self,
        snapshot: &DisplaySnapshot,
    ) -> std::ops::RangeInclusive<DisplayRow> {
        let start = self.start_anchor.to_display_point(snapshot).row();
        let current = self.current_anchor.to_display_point(snapshot).row();

        (start..=current).sorted()
    }
}

impl StoredReviewComment {
    fn new(
        id: usize,
        comment: String,
        range: Range<Anchor>,
        snapshot: &MultiBufferSnapshot,
    ) -> Self {
        let excerpt = snapshot.text_for_range(range.clone()).collect::<String>();
        Self {
            id,
            comment,
            range,
            excerpt: excerpt.into(),
            agent_reply: None,
            is_editing: false,
            status: ReviewCommentStatus::Draft,
        }
    }

    fn current_range(&self, snapshot: &MultiBufferSnapshot) -> Option<Range<Point>> {
        if !self.range.start.is_valid(snapshot) || !self.range.end.is_valid(snapshot) {
            return None;
        }

        let start = self.range.start.to_point(snapshot);
        let end = self.range.end.to_point(snapshot);
        (start <= end
            && snapshot
                .text_for_range(start..end)
                .flat_map(str::bytes)
                .eq(self.excerpt.bytes()))
        .then_some(start..end)
    }
}

impl Editor {
    pub fn diff_hunks_in_ranges<'a>(
        &'a self,
        ranges: &'a [Range<Anchor>],
        buffer: &'a MultiBufferSnapshot,
    ) -> impl 'a + Iterator<Item = MultiBufferDiffHunk> {
        ranges.iter().flat_map(move |range| {
            let end_excerpt = buffer.excerpt_containing(range.end..range.end);
            let range = range.to_point(buffer);
            let mut peek_end = range.end;
            if range.end.row < buffer.max_row().0 {
                peek_end = Point::new(range.end.row + 1, 0);
            }
            buffer
                .diff_hunks_in_range(range.start..peek_end)
                .filter(move |hunk| {
                    if let Some((_, excerpt_range)) = &end_excerpt
                        && let Some(end_anchor) =
                            buffer.anchor_in_excerpt(excerpt_range.context.end)
                        && let Some(hunk_end_anchor) =
                            buffer.anchor_in_excerpt(hunk.excerpt_range.context.end)
                        && hunk_end_anchor.cmp(&end_anchor, buffer).is_gt()
                    {
                        false
                    } else {
                        true
                    }
                })
        })
    }

    fn resolve_diff_hunks(
        &self,
        hunks: Vec<MultiBufferDiffHunk>,
        cx: &App,
    ) -> Vec<ResolvedDiffHunks> {
        let multibuffer = self.buffer().read(cx);
        let chunk_by = hunks.into_iter().chunk_by(|hunk| hunk.buffer_id);
        let mut resolved = Vec::new();

        for (source_buffer_id, hunks) in &chunk_by {
            let Some(diff) = multibuffer.diff_for(source_buffer_id) else {
                continue;
            };
            let diff_snapshot = diff.read(cx).snapshot(cx);
            let main_buffer_id = diff_snapshot.buffer_id();
            let buffer = multibuffer.buffer(main_buffer_id).or_else(|| {
                self.project
                    .as_ref()
                    .and_then(|project| project.read(cx).buffer_for_id(main_buffer_id, cx))
            });
            let mut resolved_hunks = Vec::new();

            for hunk in hunks {
                if hunk.buffer_id == main_buffer_id {
                    resolved_hunks.push(ResolvedDiffHunk {
                        buffer_range: hunk.buffer_range,
                        diff_base_byte_range: hunk.diff_base_byte_range.start.0
                            ..hunk.diff_base_byte_range.end.0,
                        status: hunk.status,
                    });
                } else {
                    let diff_base_byte_range =
                        hunk.diff_base_byte_range.start.0..hunk.diff_base_byte_range.end.0;
                    let Some(hunk) = diff_snapshot
                        .hunks_intersecting_base_text_range(
                            diff_base_byte_range.clone(),
                            diff_snapshot.buffer_snapshot(),
                        )
                        .find(|hunk| hunk.diff_base_byte_range == diff_base_byte_range)
                    else {
                        continue;
                    };
                    let kind = if hunk.buffer_range.start == hunk.buffer_range.end {
                        DiffHunkStatusKind::Deleted
                    } else if hunk.diff_base_byte_range.is_empty() {
                        DiffHunkStatusKind::Added
                    } else {
                        DiffHunkStatusKind::Modified
                    };
                    resolved_hunks.push(ResolvedDiffHunk {
                        buffer_range: hunk.buffer_range,
                        diff_base_byte_range: hunk.diff_base_byte_range,
                        status: DiffHunkStatus {
                            kind,
                            secondary: hunk.secondary_status,
                        },
                    });
                }
            }

            if !resolved_hunks.is_empty() {
                resolved.push(ResolvedDiffHunks {
                    diff,
                    buffer,
                    hunks: resolved_hunks,
                });
            }
        }

        resolved
    }

    pub fn diff_hunk_renderer(&self) -> Arc<dyn DiffHunkRenderer> {
        self.diff_hunk_renderer
            .clone()
            .unwrap_or_else(|| Arc::new(DefaultDiffHunkRenderer))
    }

    pub fn set_diff_hunk_renderer(
        &mut self,
        renderer: Option<Arc<dyn DiffHunkRenderer>>,
        cx: &mut Context<Self>,
    ) {
        let had_renderer = self.diff_hunk_renderer.is_some();
        let has_renderer = renderer.is_some();
        self.diff_hunk_renderer = renderer;

        if !had_renderer && has_renderer {
            self.load_diff_task.take();
        } else if had_renderer && !has_renderer {
            self.buffer.update(cx, |buffer, cx| {
                buffer.set_all_diff_hunks_collapsed(cx);
            });

            if let Some(project) = self.project.clone() {
                self.load_diff_task = Some(
                    self.update_uncommitted_diff_for_buffer(
                        &project,
                        self.buffer.read(cx).all_buffers(),
                        cx,
                    )
                    .shared(),
                );
            }
        }

        cx.notify();
    }

    pub(crate) fn set_diff_hunk_action_target(&mut self, target: Option<WeakEntity<Editor>>) {
        self.diff_hunk_action_target = target;
    }

    pub fn git_blame_inline_enabled(&self) -> bool {
        self.git_blame_inline_enabled
    }

    pub fn blame(&self) -> Option<&Entity<GitBlame>> {
        self.blame.as_ref()
    }

    pub fn active_git_blame_entry(&self, cx: &mut App) -> Option<BlameEntry> {
        if !self.show_git_blame_inline
            || self.newest_selection_head_on_empty_line(cx)
            || !self.has_blame_entries(cx)
        {
            return None;
        }

        let blame = self.blame.as_ref()?;
        let snapshot = self.display_snapshot(cx);
        let cursor = self.selections.newest::<Point>(&snapshot).head();
        let (buffer, point) = snapshot.buffer_snapshot().point_to_buffer_point(cursor)?;

        blame
            .update(cx, |blame, cx| {
                blame
                    .blame_for_rows(
                        &[RowInfo {
                            buffer_id: Some(buffer.remote_id()),
                            buffer_row: Some(point.row),
                            ..Default::default()
                        }],
                        cx,
                    )
                    .next()
            })
            .flatten()
            .map(|(_, entry)| entry)
    }

    pub fn show_git_blame_gutter(&self) -> bool {
        self.show_git_blame_gutter
    }

    pub fn expand_selected_diff_hunks(&mut self, cx: &mut Context<Self>) {
        let ranges: Vec<_> = self
            .selections
            .disjoint_anchors()
            .iter()
            .map(|s| s.range())
            .collect();
        self.buffer
            .update(cx, |buffer, cx| buffer.expand_diff_hunks(ranges, cx))
    }

    pub fn toggle_git_blame(
        &mut self,
        _: &::git::Blame,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.show_git_blame_gutter = !self.show_git_blame_gutter;

        if self.show_git_blame_gutter && !self.has_blame_entries(cx) {
            self.start_git_blame(true, window, cx);
        }

        cx.notify();
    }

    pub fn toggle_git_blame_inline(
        &mut self,
        _: &ToggleGitBlameInline,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.toggle_git_blame_inline_internal(true, window, cx);
        cx.notify();
    }

    /// Hides the inline blame popover element, in case it's already visible, or
    /// interrupts the task meant to show it, in case the task is running.
    ///
    /// When `ignore_timeout` is set to `true`, the popover is hidden
    /// immediately, otherwise it'll be hidden after a short delay.
    ///
    /// Returns `true` if the popover was visible and was hidden, `false`
    /// otherwise.
    pub fn hide_blame_popover(&mut self, ignore_timeout: bool, cx: &mut Context<Self>) -> bool {
        self.inline_blame_popover_show_task.take();

        if let Some(state) = &mut self.inline_blame_popover {
            if ignore_timeout {
                self.inline_blame_popover.take();
                cx.notify();
            } else {
                state.hide_task = Some(cx.spawn(async move |editor, cx| {
                    cx.background_executor()
                        .timer(std::time::Duration::from_millis(100))
                        .await;

                    editor
                        .update(cx, |editor, cx| {
                            editor.inline_blame_popover.take();
                            cx.notify();
                        })
                        .ok();
                }));
            }

            true
        } else {
            false
        }
    }

    pub fn git_restore(&mut self, _: &Restore, window: &mut Window, cx: &mut Context<Self>) {
        if self.read_only(cx) {
            return;
        }
        let selections = self
            .selections
            .all(&self.display_snapshot(cx))
            .into_iter()
            .map(|s| s.range())
            .collect();
        self.restore_hunks_in_ranges(selections, window, cx);
    }

    pub fn status_for_buffer_id(&self, buffer_id: BufferId, cx: &App) -> Option<FileStatus> {
        if let Some(status) = self
            .addons
            .iter()
            .find_map(|(_, addon)| addon.override_status_for_buffer_id(buffer_id, cx))
        {
            return Some(status);
        }
        self.project
            .as_ref()?
            .read(cx)
            .status_for_buffer_id(buffer_id, cx)
    }

    pub fn go_to_hunk_before_or_after_position(
        &mut self,
        snapshot: &EditorSnapshot,
        position: Point,
        direction: Direction,
        wrap_around: bool,
        window: &mut Window,
        cx: &mut Context<Editor>,
    ) {
        let row = if direction == Direction::Next {
            self.hunk_after_position(snapshot, position, wrap_around)
                .map(|hunk| hunk.row_range.start)
        } else {
            self.hunk_before_position(snapshot, position, wrap_around)
        };

        if let Some(row) = row {
            let destination = Point::new(row.0, 0);
            let autoscroll = Autoscroll::center();

            self.unfold_ranges(&[destination..destination], false, false, cx);
            self.change_selections(SelectionEffects::scroll(autoscroll), window, cx, |s| {
                s.select_ranges([destination..destination]);
            });
        }
    }

    pub fn set_expand_all_diff_hunks(&mut self, cx: &mut App) {
        self.buffer.update(cx, |buffer, cx| {
            buffer.set_all_diff_hunks_expanded(cx);
        });
    }

    pub fn expand_all_diff_hunks(
        &mut self,
        _: &ExpandAllDiffHunks,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.buffer.update(cx, |buffer, cx| {
            buffer.expand_diff_hunks(vec![Anchor::Min..Anchor::Max], cx)
        });
    }

    pub fn show_diff_review_overlay(
        &mut self,
        display_range: Range<DisplayRow>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Range { start, end } = display_range.sorted();

        let buffer_snapshot = self.buffer.read(cx).snapshot(cx);
        let editor_snapshot = self.snapshot(window, cx);

        // Convert display rows to multibuffer points
        let start_point = editor_snapshot
            .display_snapshot
            .display_point_to_point(start.as_display_point(), Bias::Left);
        let end_point = editor_snapshot
            .display_snapshot
            .display_point_to_point(end.as_display_point(), Bias::Left);
        let end_multi_buffer_row = MultiBufferRow(end_point.row);

        // Create anchor range for the selected lines (start of first line to end of last line)
        let line_end = Point::new(
            end_point.row,
            buffer_snapshot.line_len(end_multi_buffer_row),
        );
        let anchor_range =
            buffer_snapshot.anchor_after(start_point)..buffer_snapshot.anchor_before(line_end);

        // Compute the hunk key for this display row
        let file_path = buffer_snapshot
            .file_at(start_point)
            .map(|file: &Arc<dyn language::File>| file.path().clone())
            .unwrap_or_else(|| Arc::from(util::rel_path::RelPath::empty()));
        let hunk_start_anchor = buffer_snapshot.anchor_before(start_point);
        let hunk_key = DiffHunkKey {
            file_path,
            hunk_start_anchor,
        };

        // Get the current user's avatar URI from the project's user_store
        let user_avatar_uri = self.project.as_ref().and_then(|project| {
            let user_store = project.read(cx).user_store();
            user_store
                .read(cx)
                .current_user()
                .map(|user| user.avatar_uri.clone())
        });

        // Create anchor at the end of the last row so the block appears immediately below it
        // Use multibuffer coordinates for anchor creation
        let line_len = buffer_snapshot.line_len(end_multi_buffer_row);
        let anchor = buffer_snapshot.anchor_after(Point::new(end_multi_buffer_row.0, line_len));

        // Create the prompt editor for the review input
        let prompt_editor = cx.new(|cx| {
            let mut editor = Editor::single_line(window, cx);
            editor.set_placeholder_text("Add a review comment...", window, cx);
            editor
        });

        // Register the Newline action on the prompt editor to submit the review
        let parent_editor = cx.entity().downgrade();
        let subscription = prompt_editor.update(cx, |prompt_editor, _cx| {
            prompt_editor.register_action({
                let parent_editor = parent_editor.clone();
                move |_: &crate::actions::Newline, window, cx| {
                    if let Some(editor) = parent_editor.upgrade() {
                        editor.update(cx, |editor, cx| {
                            editor.submit_diff_review_comment(window, cx);
                        });
                    }
                }
            })
        });

        let editor_handle = cx.entity().downgrade();
        let block = BlockProperties {
            style: BlockStyle::Sticky,
            placement: BlockPlacement::Below(anchor),
            height: Some(2),
            render: Arc::new(move |cx| Self::render_diff_review_overlay(&editor_handle, cx)),
            priority: 0,
        };

        let block_ids = self.insert_blocks([block], None, cx);
        let Some(block_id) = block_ids.into_iter().next() else {
            log::error!("Failed to insert diff review overlay block");
            return;
        };

        self.diff_review_overlays.push(DiffReviewOverlay {
            anchor_range,
            block_id,
            prompt_editor: prompt_editor.clone(),
            hunk_key,
            comment_id: None,
            comments_expanded: true,
            inline_edit: None,
            user_avatar_uri,
            _subscription: subscription,
        });

        // Focus the prompt editor
        let focus_handle = prompt_editor.focus_handle(cx);
        window.focus(&focus_handle, cx);

        cx.notify();
    }

    pub fn submit_diff_review_comment(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(overlay) = self.diff_review_overlays.iter().find(|overlay| {
            overlay.comment_id.is_none()
                && overlay.prompt_editor.focus_handle(cx).is_focused(window)
        }) {
            self.submit_diff_review_overlay(overlay.block_id, window, cx);
        }
    }

    fn submit_diff_review_overlay(
        &mut self,
        block_id: CustomBlockId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self
            .diff_review_overlays
            .iter()
            .position(|overlay| overlay.block_id == block_id && overlay.comment_id.is_none())
        else {
            return;
        };
        let overlay = &self.diff_review_overlays[index];
        let comment = overlay.prompt_editor.read(cx).text(cx).trim().to_string();
        if comment.is_empty() {
            return;
        }

        let restore_focus = overlay.prompt_editor.focus_handle(cx).is_focused(window);
        let id = self.add_review_comment(
            overlay.hunk_key.clone(),
            comment,
            overlay.anchor_range.clone(),
            cx,
        );
        self.diff_review_overlays[index].comment_id = Some(id);
        if restore_focus {
            window.focus(&self.focus_handle, cx);
        }
    }

    pub fn diff_review_prompt_editor(&self) -> Option<&Entity<Editor>> {
        self.diff_review_overlays
            .iter()
            .rev()
            .find(|overlay| overlay.comment_id.is_none())
            .map(|overlay| &overlay.prompt_editor)
    }

    /// Sets whether the comments section is expanded in the diff review overlay.
    /// This is primarily used for testing.
    pub fn set_diff_review_comments_expanded(&mut self, expanded: bool, cx: &mut Context<Self>) {
        for overlay in &mut self.diff_review_overlays {
            overlay.comments_expanded = expanded;
        }
        cx.notify();
    }

    /// Returns the total count of sendable review comments across all hunks.
    pub(super) fn total_review_comment_count(&self) -> usize {
        self.stored_review_comments
            .iter()
            .flat_map(|(_, comments)| comments.iter())
            .filter(|comment| comment.status.is_sendable() && !comment.is_editing)
            .count()
    }

    pub fn review_feedback(&self, cx: &App) -> Vec<(usize, ReviewFeedback)> {
        let snapshot = self.buffer.read(cx).snapshot(cx);
        self.stored_review_comments
            .iter()
            .flat_map(|(hunk, comments)| {
                comments.iter().filter_map(|comment| {
                    if !comment.status.is_sendable() || comment.is_editing {
                        return None;
                    }
                    let Range {
                        start: start_point,
                        end: end_point,
                    } = comment.current_range(&snapshot)?;
                    let buffer_ranges = snapshot.range_to_buffer_ranges(start_point..end_point);
                    let (Some((first_buffer, first_range, _)), Some((last_buffer, last_range, _))) =
                        (buffer_ranges.first(), buffer_ranges.last())
                    else {
                        log::debug!(
                            "Skipping review comment {} for {} because its buffer range could not be resolved ({start_point:?}..{end_point:?})",
                            comment.id,
                            hunk.file_path.as_unix_str(),
                        );
                        return None;
                    };
                    if first_buffer.remote_id() != last_buffer.remote_id() {
                        return None;
                    }
                    let start_line = first_buffer.offset_to_point(first_range.start.0).row;
                    let end_line = last_buffer.offset_to_point(last_range.end.0).row;
                    let worktree_name = snapshot
                        .file_at(start_point)
                        .and_then(|file| {
                            self.project
                                .as_ref()?
                                .read(cx)
                                .worktree_for_id(file.worktree_id(cx), cx)
                        })
                        .map(|worktree| worktree.read(cx).root_name().as_unix_str().to_string());
                    Some((
                        comment.id,
                        ReviewFeedback {
                            worktree_name,
                            file_path: hunk.file_path.as_unix_str().to_string(),
                            start_line: start_line.saturating_add(1),
                            end_line: end_line.saturating_add(1),
                            excerpt: comment.excerpt.to_string(),
                            comment: comment.comment.clone(),
                            status: comment.status,
                        },
                    ))
                })
            })
            .collect()
    }

    /// Builds a provider-agnostic review session from the local draft comments.
    pub fn review_session(&self, cx: &App) -> ReviewSession {
        let feedback = self.review_feedback(cx);
        ReviewSession {
            source: ReviewSessionSource::LocalDiff,
            threads: feedback
                .into_iter()
                .map(|(id, item)| ReviewThread {
                    id: format!("local-{id}"),
                    file_path: item.file_path,
                    start_line: item.start_line,
                    end_line: item.end_line,
                    excerpt: item.excerpt,
                    status: item.status,
                    comments: vec![ReviewComment {
                        id: format!("local-comment-{id}"),
                        author: "You".to_string(),
                        body: item.comment,
                        is_draft: item.status == ReviewCommentStatus::Draft,
                    }],
                })
                .collect(),
        }
    }

    pub fn review_worktree(&self, comment_ids: &[usize], cx: &App) -> Option<Arc<std::path::Path>> {
        let project = self.project.as_ref()?.read(cx);
        let snapshot = self.buffer.read(cx).snapshot(cx);
        let mut worktree = None;

        for (_, comments) in &self.stored_review_comments {
            for comment in comments
                .iter()
                .filter(|comment| comment_ids.contains(&comment.id))
            {
                let range = comment.current_range(&snapshot)?;
                let file = snapshot.file_at(range.start)?;
                let root = project
                    .worktree_for_id(file.worktree_id(cx), cx)?
                    .read(cx)
                    .abs_path()
                    .clone();
                if worktree.as_ref().is_some_and(|existing| existing != &root) {
                    return None;
                }
                worktree = Some(root);
            }
        }
        worktree
    }

    pub fn begin_diff_review(&mut self, comment_ids: &[usize], cx: &mut Context<Self>) {
        for (_, comments) in &mut self.stored_review_comments {
            for comment in comments
                .iter_mut()
                .filter(|comment| comment_ids.contains(&comment.id))
            {
                comment.status = ReviewCommentStatus::InReview;
            }
        }
        cx.emit(EditorEvent::ReviewCommentsChanged {
            total_count: self.total_review_comment_count(),
        });
        cx.notify();
    }

    pub fn finish_diff_review(
        &mut self,
        comment_ids: &[usize],
        agent: SharedString,
        replies: Option<&HashMap<usize, String>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let snapshot = self.buffer.read(cx).snapshot(cx);
        for (_, comments) in &mut self.stored_review_comments {
            for comment in comments.iter_mut().filter(|comment| {
                comment.status == ReviewCommentStatus::InReview && comment_ids.contains(&comment.id)
            }) {
                comment.status = if comment.current_range(&snapshot).is_none() {
                    ReviewCommentStatus::Outdated
                } else if replies.is_some() {
                    ReviewCommentStatus::Sent
                } else {
                    ReviewCommentStatus::Draft
                };
                if let Some(reply) = replies.and_then(|replies| replies.get(&comment.id)) {
                    comment.agent_reply = Some((agent.clone(), reply.clone()));
                }
            }
        }
        cx.emit(EditorEvent::ReviewCommentsChanged {
            total_count: self.total_review_comment_count(),
        });
        cx.notify();
    }

    /// Adds a new review comment to a specific hunk.
    pub(super) fn add_review_comment(
        &mut self,
        hunk_key: DiffHunkKey,
        comment: String,
        anchor_range: Range<Anchor>,
        cx: &mut Context<Self>,
    ) -> usize {
        let id = self.next_review_comment_id;
        self.next_review_comment_id += 1;

        let snapshot = self.buffer.read(cx).snapshot(cx);
        let stored_comment = StoredReviewComment::new(id, comment, anchor_range, &snapshot);

        let key_point = hunk_key.hunk_start_anchor.to_point(&snapshot);

        // Find existing entry for this hunk or add a new one
        if let Some((_, comments)) = self.stored_review_comments.iter_mut().find(|(k, _)| {
            k.file_path == hunk_key.file_path
                && k.hunk_start_anchor.to_point(&snapshot) == key_point
        }) {
            comments.push(stored_comment);
        } else {
            self.stored_review_comments
                .push((hunk_key, vec![stored_comment]));
        }

        cx.emit(EditorEvent::ReviewCommentsChanged {
            total_count: self.total_review_comment_count(),
        });
        cx.notify();
        id
    }

    pub(super) fn blame_hover(
        &mut self,
        _: &BlameHover,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let just_started = self.blame.is_none();
        if just_started {
            self.start_git_blame(true, window, cx);
        }
        let Some(blame) = self.blame.as_ref() else {
            return;
        };

        if just_started && !blame.read(cx).has_generated_entries() {
            let subscription = cx.observe_in(blame, window, |editor, blame, window, cx| {
                if blame.read(cx).has_generated_entries() {
                    editor.pending_blame_hover_observation.take();
                    editor.show_blame_hover_popover(window, cx);
                }
            });
            self.pending_blame_hover_observation = Some(subscription);
            return;
        }

        self.show_blame_hover_popover(window, cx);
    }

    fn show_blame_hover_popover(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let snapshot = self.snapshot(window, cx);
        let cursor = self
            .selections
            .newest::<Point>(&snapshot.display_snapshot)
            .head();
        let Some((buffer, point)) = snapshot.buffer_snapshot().point_to_buffer_point(cursor) else {
            return;
        };

        let Some(blame) = self.blame.as_ref() else {
            return;
        };

        let row_info = RowInfo {
            buffer_id: Some(buffer.remote_id()),
            buffer_row: Some(point.row),
            ..Default::default()
        };
        let Some((buffer, blame_entry)) = blame
            .update(cx, |blame, cx| blame.blame_for_rows(&[row_info], cx).next())
            .flatten()
        else {
            return;
        };

        let anchor = self.selections.newest_anchor().head();
        let position = self.to_pixel_point(anchor, &snapshot, window, cx);
        if let (Some(position), Some(last_bounds)) = (position, self.last_bounds) {
            self.show_blame_popover(
                buffer,
                &blame_entry,
                position + last_bounds.origin,
                true,
                cx,
            );
        };
    }

    pub(super) fn restore_file(
        &mut self,
        _: &::git::RestoreFile,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.read_only(cx) {
            return;
        }
        let mut buffer_ids = HashSet::default();
        let snapshot = self.buffer().read(cx).snapshot(cx);
        for selection in self
            .selections
            .all::<MultiBufferOffset>(&self.display_snapshot(cx))
        {
            buffer_ids.extend(snapshot.buffer_ids_for_range(selection.range()))
        }

        let ranges = buffer_ids
            .into_iter()
            .flat_map(|buffer_id| snapshot.range_for_buffer(buffer_id))
            .collect::<Vec<_>>();

        self.restore_hunks_in_ranges(ranges, window, cx);
    }

    /// Restores the diff hunks in the editor's selections and moves the cursor
    /// to the next diff hunk. Wraps around to the beginning of the buffer if
    /// not all diff hunks are expanded.
    pub(super) fn restore_and_next(
        &mut self,
        _: &::git::RestoreAndNext,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.read_only(cx) {
            return;
        }
        let selections = self
            .selections
            .all(&self.display_snapshot(cx))
            .into_iter()
            .map(|selection| selection.range())
            .collect();

        self.restore_hunks_in_ranges(selections, window, cx);

        let all_diff_hunks_expanded = self.buffer().read(cx).all_diff_hunks_expanded();
        let wrap_around = !all_diff_hunks_expanded;
        let snapshot = self.snapshot(window, cx);
        let position = self
            .selections
            .newest::<Point>(&snapshot.display_snapshot)
            .head();

        self.go_to_hunk_before_or_after_position(
            &snapshot,
            position,
            Direction::Next,
            wrap_around,
            window,
            cx,
        );
    }

    fn restore_diff_hunks(&mut self, hunks: Vec<ResolvedDiffHunks>, cx: &mut Context<Self>) {
        let mut revert_changes = Vec::new();
        for hunks in hunks {
            let Some(buffer) = hunks.buffer else {
                continue;
            };
            let diff_snapshot = hunks.diff.read(cx).snapshot(cx);
            let changes = hunks
                .hunks
                .into_iter()
                .filter_map(|hunk| {
                    if hunk.diff_base_byte_range == (0..0)
                        && hunk.buffer_range.start.is_min()
                        && hunk.buffer_range.end.is_max()
                    {
                        return None;
                    }
                    let original_text = diff_snapshot
                        .base_text()
                        .as_rope()
                        .slice(hunk.diff_base_byte_range.start..hunk.diff_base_byte_range.end);
                    Some((hunk.buffer_range, original_text))
                })
                .collect::<Vec<_>>();
            if !changes.is_empty() {
                revert_changes.push((buffer, changes));
            }
        }

        for (buffer, changes) in revert_changes {
            buffer.update(cx, |buffer, cx| {
                buffer.edit(
                    changes
                        .into_iter()
                        .map(|(range, text)| (range, text.to_string())),
                    None,
                    cx,
                );
            });
        }
    }

    pub(super) fn go_to_next_hunk(
        &mut self,
        _: &GoToHunk,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let snapshot = self.snapshot(window, cx);
        let selection = self.selections.newest::<Point>(&self.display_snapshot(cx));
        self.go_to_hunk_before_or_after_position(
            &snapshot,
            selection.head(),
            Direction::Next,
            true,
            window,
            cx,
        );
    }

    pub(super) fn collapse_all_diff_hunks(
        &mut self,
        _: &CollapseAllDiffHunks,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.buffer.update(cx, |buffer, cx| {
            buffer.collapse_diff_hunks(vec![Anchor::Min..Anchor::Max], cx)
        });
    }

    pub fn toggle_all_diff_hunks(
        &mut self,
        _: &ToggleAllDiffHunks,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.has_any_expanded_diff_hunks(cx) {
            self.collapse_all_diff_hunks(&CollapseAllDiffHunks, window, cx);
        } else {
            self.expand_all_diff_hunks(&ExpandAllDiffHunks, window, cx);
        }
    }

    pub(super) fn toggle_selected_diff_hunks(
        &mut self,
        _: &ToggleSelectedDiffHunks,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let ranges: Vec<_> = self
            .selections
            .disjoint_anchors()
            .iter()
            .map(|s| s.range())
            .collect();
        self.toggle_diff_hunks_in_ranges(ranges, cx);
    }

    pub(super) fn show_diff_review_button(&self) -> bool {
        self.show_diff_review_button
    }

    pub(super) fn render_diff_review_button(
        &self,
        display_row: DisplayRow,
        width: Pixels,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let text_color = cx.theme().colors().text;
        let icon_color = cx.theme().colors().icon_accent;

        h_flex()
            .id("diff_review_button")
            .cursor_pointer()
            .w(width - px(1.))
            .h(relative(0.9))
            .justify_center()
            .rounded_sm()
            .border_1()
            .border_color(text_color.opacity(0.1))
            .bg(text_color.opacity(0.15))
            .hover(|s| {
                s.bg(icon_color.opacity(0.4))
                    .border_color(icon_color.opacity(0.5))
            })
            .child(Icon::new(IconName::Plus).size(IconSize::Small))
            .tooltip(Tooltip::text("Add Review (drag to select multiple lines)"))
            .on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(move |editor, _event: &gpui::MouseDownEvent, window, cx| {
                    editor.start_diff_review_drag(display_row, window, cx);
                }),
            )
    }

    pub(super) fn start_diff_review_drag(
        &mut self,
        display_row: DisplayRow,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let snapshot = self.snapshot(window, cx);
        let point = snapshot
            .display_snapshot
            .display_point_to_point(DisplayPoint::new(display_row, 0), Bias::Left);
        let anchor = snapshot.buffer_snapshot().anchor_before(point);
        self.diff_review_drag_state = Some(DiffReviewDragState {
            start_anchor: anchor,
            current_anchor: anchor,
        });
        cx.notify();
    }

    pub(super) fn update_diff_review_drag(
        &mut self,
        display_row: DisplayRow,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.diff_review_drag_state.is_none() {
            return;
        }
        let snapshot = self.snapshot(window, cx);
        let point = snapshot
            .display_snapshot
            .display_point_to_point(display_row.as_display_point(), Bias::Left);
        let anchor = snapshot.buffer_snapshot().anchor_before(point);
        if let Some(drag_state) = &mut self.diff_review_drag_state {
            drag_state.current_anchor = anchor;
            cx.notify();
        }
    }

    pub(super) fn end_diff_review_drag(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(drag_state) = self.diff_review_drag_state.take() {
            let snapshot = self.snapshot(window, cx);
            let range = drag_state.row_range(&snapshot.display_snapshot);
            self.show_diff_review_overlay(*range.start()..*range.end(), window, cx);
        }
        cx.notify();
    }

    pub(super) fn cancel_diff_review_drag(&mut self, cx: &mut Context<Self>) {
        self.diff_review_drag_state = None;
        cx.notify();
    }

    /// Dismisses all diff review overlays.
    #[cfg(test)]
    pub(super) fn dismiss_all_diff_review_overlays(&mut self, cx: &mut Context<Self>) {
        if self.diff_review_overlays.is_empty() {
            return;
        }
        let block_ids: HashSet<_> = self
            .diff_review_overlays
            .drain(..)
            .map(|overlay| overlay.block_id)
            .collect();
        self.remove_blocks(block_ids, None, cx);
        cx.notify();
    }

    /// Action handler for SubmitDiffReviewComment.
    pub(super) fn submit_diff_review_comment_action(
        &mut self,
        _: &SubmitDiffReviewComment,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.submit_diff_review_comment(window, cx);
    }

    /// Returns comments for a specific hunk, ordered by creation time.
    #[cfg(test)]
    pub(super) fn comments_for_hunk<'a>(
        &'a self,
        key: &DiffHunkKey,
        snapshot: &MultiBufferSnapshot,
    ) -> &'a [StoredReviewComment] {
        let key_point = key.hunk_start_anchor.to_point(snapshot);
        self.stored_review_comments
            .iter()
            .find(|(k, _)| {
                k.file_path == key.file_path && k.hunk_start_anchor.to_point(snapshot) == key_point
            })
            .map(|(_, comments)| comments.as_slice())
            .unwrap_or(&[])
    }

    #[cfg(test)]
    pub(super) fn hunk_comment_count(
        &self,
        key: &DiffHunkKey,
        snapshot: &MultiBufferSnapshot,
    ) -> usize {
        self.comments_for_hunk(key, snapshot).len()
    }

    fn review_comment(&self, comment_id: usize) -> Option<&StoredReviewComment> {
        self.stored_review_comments
            .iter()
            .flat_map(|(_, comments)| comments)
            .find(|comment| comment.id == comment_id)
    }

    /// Removes a review comment by ID from any hunk.
    pub(super) fn remove_review_comment(&mut self, id: usize, cx: &mut Context<Self>) -> bool {
        for (_, comments) in self.stored_review_comments.iter_mut() {
            if let Some(index) = comments.iter().position(|c| c.id == id) {
                comments.remove(index);
                cx.emit(EditorEvent::ReviewCommentsChanged {
                    total_count: self.total_review_comment_count(),
                });
                cx.notify();
                return true;
            }
        }
        false
    }

    /// Updates a review comment's text by ID.
    pub(super) fn update_review_comment(
        &mut self,
        id: usize,
        new_comment: String,
        cx: &mut Context<Self>,
    ) -> bool {
        for (_, comments) in self.stored_review_comments.iter_mut() {
            if let Some(comment) = comments.iter_mut().find(|c| c.id == id) {
                comment.comment = new_comment;
                comment.is_editing = false;
                cx.emit(EditorEvent::ReviewCommentsChanged {
                    total_count: self.total_review_comment_count(),
                });
                cx.notify();
                return true;
            }
        }
        false
    }

    /// Sets a comment's editing state.
    pub(super) fn set_comment_editing(
        &mut self,
        id: usize,
        is_editing: bool,
        cx: &mut Context<Self>,
    ) {
        for (_, comments) in &mut self.stored_review_comments {
            if let Some(comment) = comments.iter_mut().find(|c| c.id == id) {
                comment.is_editing = is_editing;
                cx.emit(EditorEvent::ReviewCommentsChanged {
                    total_count: self.total_review_comment_count(),
                });
                cx.notify();
                return;
            }
        }
    }

    pub(super) fn cleanup_orphaned_review_comments(&mut self, cx: &mut Context<Self>) {
        if self.stored_review_comments.is_empty() {
            return;
        }

        let snapshot = self.buffer.read(cx).snapshot(cx);
        let mut changed = false;

        for (_, comments) in &mut self.stored_review_comments {
            for comment in comments {
                if matches!(
                    comment.status,
                    ReviewCommentStatus::Draft
                        | ReviewCommentStatus::Queued
                        | ReviewCommentStatus::Sent
                ) && comment.current_range(&snapshot).is_none()
                {
                    comment.status = ReviewCommentStatus::Outdated;
                    changed = true;
                }
            }
        }

        if changed {
            cx.emit(EditorEvent::ReviewCommentsChanged {
                total_count: self.total_review_comment_count(),
            });
            cx.notify();
        }
    }

    pub(super) fn dismiss_focused_diff_review_overlay(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        for overlay in &self.diff_review_overlays {
            if overlay.comment_id.is_none()
                && overlay.prompt_editor.focus_handle(cx).is_focused(window)
            {
                self.close_diff_review_overlay(overlay.block_id, window, cx);
                return true;
            }
            if let Some(comment_id) = overlay.comment_id
                && overlay
                    .inline_edit
                    .as_ref()
                    .is_some_and(|(editor, _)| editor.focus_handle(cx).is_focused(window))
            {
                self.cancel_edit_review_comment(comment_id, window, cx);
                return true;
            }
        }
        false
    }

    fn remove_diff_review_overlay(
        &mut self,
        block_id: CustomBlockId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self
            .diff_review_overlays
            .iter()
            .position(|overlay| overlay.block_id == block_id)
        else {
            return;
        };
        let overlay = self.diff_review_overlays.remove(index);
        if overlay.prompt_editor.focus_handle(cx).is_focused(window)
            || overlay
                .inline_edit
                .as_ref()
                .is_some_and(|(editor, _)| editor.focus_handle(cx).is_focused(window))
        {
            window.focus(&self.focus_handle, cx);
        }
        self.remove_blocks(HashSet::from_iter([block_id]), None, cx);
    }

    pub(super) fn close_diff_review_overlay(
        &mut self,
        block_id: CustomBlockId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(overlay) = self
            .diff_review_overlays
            .iter_mut()
            .find(|overlay| overlay.block_id == block_id)
        else {
            return;
        };
        if let Some(comment_id) = overlay.comment_id {
            overlay.comments_expanded = false;
            self.cancel_edit_review_comment(comment_id, window, cx);
            cx.notify();
        } else {
            self.remove_diff_review_overlay(block_id, window, cx);
        }
    }

    pub(super) fn toggle_review_comments_expanded(
        &mut self,
        _: &ToggleReviewCommentsExpanded,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(overlay) = self
            .diff_review_overlays
            .iter()
            .find(|overlay| overlay.comment_id.is_some())
        {
            self.toggle_diff_review_overlay(overlay.block_id, window, cx);
        }
    }

    fn toggle_diff_review_overlay(
        &mut self,
        block_id: CustomBlockId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(overlay) = self
            .diff_review_overlays
            .iter_mut()
            .find(|overlay| overlay.block_id == block_id)
        else {
            return;
        };
        if overlay.comments_expanded {
            self.close_diff_review_overlay(block_id, window, cx);
        } else {
            overlay.comments_expanded = true;
            cx.notify();
        }
    }

    pub(super) fn edit_review_comment(
        &mut self,
        action: &EditReviewComment,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let comment_id = action.id;
        let Some(comment) = self
            .review_comment(comment_id)
            .filter(|comment| comment.status.is_sendable())
        else {
            return;
        };
        let comment_text = comment.comment.clone();
        let Some(overlay) = self.diff_review_overlays.iter_mut().find(|overlay| {
            overlay.comment_id == Some(comment_id) && overlay.inline_edit.is_none()
        }) else {
            return;
        };

        let inline_editor = cx.new(|cx| {
            let mut editor = Editor::single_line(window, cx);
            editor.set_text(comment_text.as_str(), window, cx);
            editor.select_all(&crate::actions::SelectAll, window, cx);
            editor
        });
        let parent_editor = cx.entity().downgrade();
        let subscription = inline_editor.update(cx, |editor, _| {
            editor.register_action(move |_: &crate::actions::Newline, window, cx| {
                if let Some(parent) = parent_editor.upgrade() {
                    parent.update(cx, |editor, cx| {
                        editor.confirm_edit_review_comment(comment_id, window, cx);
                    });
                }
            })
        });

        window.focus(&inline_editor.focus_handle(cx), cx);
        overlay.comments_expanded = true;
        overlay.inline_edit = Some((inline_editor, subscription));
        self.set_comment_editing(comment_id, true, cx);
    }

    pub(super) fn confirm_edit_review_comment(
        &mut self,
        comment_id: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let new_text = self
            .diff_review_overlays
            .iter()
            .find(|overlay| overlay.comment_id == Some(comment_id))
            .and_then(|overlay| overlay.inline_edit.as_ref())
            .map(|(editor, _)| editor.read(cx).text(cx).trim().to_string());

        if let Some(text) = new_text.filter(|text| !text.is_empty()) {
            self.update_review_comment(comment_id, text, cx);
        }
        self.cancel_edit_review_comment(comment_id, window, cx);
    }

    pub(super) fn cancel_edit_review_comment(
        &mut self,
        comment_id: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(overlay) = self
            .diff_review_overlays
            .iter_mut()
            .find(|overlay| overlay.comment_id == Some(comment_id))
            && let Some((editor, _)) = overlay.inline_edit.take()
        {
            if editor.focus_handle(cx).is_focused(window) {
                window.focus(&self.focus_handle, cx);
            }
            self.set_comment_editing(comment_id, false, cx);
        }
    }

    /// Action handler for ConfirmEditReviewComment.
    pub(super) fn confirm_edit_review_comment_action(
        &mut self,
        action: &ConfirmEditReviewComment,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.confirm_edit_review_comment(action.id, window, cx);
    }

    /// Action handler for CancelEditReviewComment.
    pub(super) fn cancel_edit_review_comment_action(
        &mut self,
        action: &CancelEditReviewComment,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.cancel_edit_review_comment(action.id, window, cx);
    }

    pub(super) fn delete_review_comment(
        &mut self,
        action: &DeleteReviewComment,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.remove_review_comment(action.id, cx)
            && let Some(overlay) = self
                .diff_review_overlays
                .iter()
                .find(|overlay| overlay.comment_id == Some(action.id))
        {
            self.remove_diff_review_overlay(overlay.block_id, window, cx);
        }
    }

    pub(super) fn copy_permalink_to_line(
        &mut self,
        _: &CopyPermalinkToLine,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let permalink_task = self.get_permalink_to_line(cx);
        let workspace = self.workspace();

        cx.spawn_in(window, async move |_, cx| match permalink_task.await {
            Ok(permalink) => {
                cx.update(|_, cx| {
                    cx.write_to_clipboard(ClipboardItem::new_string(permalink.to_string()));
                })
                .ok();
            }
            Err(err) => {
                let message = format!("Failed to copy permalink to line: {err}");

                anyhow::Result::<()>::Err(err).log_err();

                if let Some(workspace) = workspace {
                    workspace
                        .update_in(cx, |workspace, _, cx| {
                            struct CopyPermalinkToLine;

                            workspace.show_toast(
                                Toast::new(
                                    NotificationId::unique::<CopyPermalinkToLine>(),
                                    message,
                                ),
                                cx,
                            )
                        })
                        .ok();
                }
            }
        })
        .detach();
    }

    pub(super) fn open_permalink_to_line(
        &mut self,
        _: &OpenPermalinkToLine,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let permalink_task = self.get_permalink_to_line(cx);
        let workspace = self.workspace();

        cx.spawn_in(window, async move |_, cx| match permalink_task.await {
            Ok(permalink) => {
                cx.update(|_, cx| {
                    cx.open_url(permalink.as_ref());
                })
                .ok();
            }
            Err(err) => {
                let message = format!("Failed to open permalink to line: {err}");

                anyhow::Result::<()>::Err(err).log_err();

                if let Some(workspace) = workspace {
                    workspace.update(cx, |workspace, cx| {
                        struct OpenPermalinkToLine;

                        workspace.show_toast(
                            Toast::new(NotificationId::unique::<OpenPermalinkToLine>(), message),
                            cx,
                        )
                    });
                }
            }
        })
        .detach();
    }

    pub(super) fn toggle_staged_selected_diff_hunks(
        &mut self,
        _: &::git::ToggleStaged,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let ranges: Vec<_> = self
            .selections
            .disjoint_anchors()
            .iter()
            .map(|s| s.range())
            .collect();
        let task = self.save_buffers_for_ranges_if_needed(&ranges, cx);
        cx.spawn_in(window, async move |this, cx| {
            task.await?;
            this.update_in(cx, |this, window, cx| {
                let snapshot = this.buffer.read(cx).snapshot(cx);
                let hunks = this.diff_hunks_in_ranges(&ranges, &snapshot).collect();
                this.apply_toggle(hunks, window, cx);
            })
        })
        .detach_and_log_err(cx);
    }

    pub(super) fn stage_and_next(
        &mut self,
        _: &::git::StageAndNext,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.do_stage_or_unstage_and_next(true, window, cx);
    }

    pub(super) fn unstage_and_next(
        &mut self,
        _: &::git::UnstageAndNext,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.do_stage_or_unstage_and_next(false, window, cx);
    }

    pub fn apply_toggle(
        &mut self,
        hunks: Vec<MultiBufferDiffHunk>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let hunks = self.resolve_diff_hunks(hunks, cx);
        if hunks.is_empty() {
            return;
        }
        if let Some(target) = self.diff_hunk_action_target.clone() {
            target
                .update(cx, |editor, cx| {
                    editor.toggle_resolved_diff_hunks(hunks, cx);
                })
                .log_err();
            return;
        }
        self.toggle_resolved_diff_hunks(hunks, cx);
    }

    fn toggle_resolved_diff_hunks(
        &mut self,
        hunks: Vec<ResolvedDiffHunks>,
        cx: &mut Context<Self>,
    ) {
        for hunks in hunks {
            let Some(operations) = hunks.diff.read(cx).operations() else {
                continue;
            };
            let stage = if !operations.supports_unstaging() {
                true
            } else if !operations.supports_staging() {
                false
            } else {
                hunks
                    .hunks
                    .iter()
                    .any(|hunk| hunk.status.has_secondary_hunk())
            };
            let ranges = hunks
                .hunks
                .into_iter()
                .map(|hunk| hunk.buffer_range)
                .collect();
            if stage {
                operations.stage(hunks.diff, hunks.buffer, ranges, cx);
            } else {
                operations.unstage(hunks.diff, hunks.buffer, ranges, cx);
            }
        }
    }

    pub fn apply_stage_or_unstage(
        &mut self,
        stage: bool,
        hunks: Vec<MultiBufferDiffHunk>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let hunks = self.resolve_diff_hunks(hunks, cx);
        if hunks.is_empty() {
            return;
        }
        if let Some(target) = self.diff_hunk_action_target.clone() {
            target
                .update(cx, |editor, cx| {
                    editor.stage_or_unstage_resolved_diff_hunks(stage, hunks, cx);
                })
                .log_err();
            return;
        }
        self.stage_or_unstage_resolved_diff_hunks(stage, hunks, cx);
    }

    fn stage_or_unstage_resolved_diff_hunks(
        &mut self,
        stage: bool,
        hunks: Vec<ResolvedDiffHunks>,
        cx: &mut Context<Self>,
    ) {
        for hunks in hunks {
            let Some(operations) = hunks.diff.read(cx).operations() else {
                continue;
            };
            if (stage && !operations.supports_staging())
                || (!stage && !operations.supports_unstaging())
            {
                continue;
            }
            let ranges = hunks
                .hunks
                .into_iter()
                .map(|hunk| hunk.buffer_range)
                .collect();
            if stage {
                operations.stage(hunks.diff, hunks.buffer, ranges, cx);
            } else {
                operations.unstage(hunks.diff, hunks.buffer, ranges, cx);
            }
        }
    }

    pub fn apply_restore(
        &mut self,
        hunks: Vec<MultiBufferDiffHunk>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let hunks = self.resolve_diff_hunks(hunks, cx);
        if hunks.is_empty() {
            return;
        }
        if let Some(target) = self.diff_hunk_action_target.clone() {
            target
                .update(cx, |editor, cx| {
                    editor.restore_resolved_diff_hunks(hunks, window, cx);
                })
                .log_err();
            return;
        }
        self.restore_resolved_diff_hunks(hunks, window, cx);
    }

    fn restore_resolved_diff_hunks(
        &mut self,
        hunks: Vec<ResolvedDiffHunks>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.read_only(cx) {
            return;
        }
        let mut restorable_hunks = Vec::new();
        for hunks in hunks {
            let Some(operations) = hunks.diff.read(cx).operations() else {
                continue;
            };
            if !operations.supports_restore() {
                continue;
            }
            if operations.supports_unstaging() {
                let ranges = hunks
                    .hunks
                    .iter()
                    .map(|hunk| hunk.buffer_range.clone())
                    .collect();
                operations.unstage(hunks.diff.clone(), hunks.buffer.clone(), ranges, cx);
            }
            restorable_hunks.push(hunks);
        }
        if restorable_hunks.is_empty() {
            return;
        }
        self.transact(window, cx, |editor, window, cx| {
            editor.restore_diff_hunks(restorable_hunks, cx);
            let selections = editor
                .selections
                .all::<MultiBufferOffset>(&editor.display_snapshot(cx));
            editor.change_selections(
                SelectionEffects::no_scroll(),
                window,
                cx,
                |selections_state| {
                    selections_state.select(selections);
                },
            );
        });
    }

    pub(super) fn clear_expanded_diff_hunks(&mut self, cx: &mut Context<Self>) -> bool {
        self.buffer.update(cx, |buffer, cx| {
            let ranges = vec![Anchor::Min..Anchor::Max];
            if !buffer.all_diff_hunks_expanded()
                && buffer.has_expanded_diff_hunks_in_ranges(&ranges, cx)
            {
                buffer.collapse_diff_hunks(ranges, cx);
                true
            } else {
                false
            }
        })
    }

    pub(super) fn has_any_expanded_diff_hunks(&self, cx: &App) -> bool {
        if self.buffer.read(cx).all_diff_hunks_expanded() {
            return true;
        }
        let ranges = vec![Anchor::Min..Anchor::Max];
        self.buffer
            .read(cx)
            .has_expanded_diff_hunks_in_ranges(&ranges, cx)
    }

    pub(super) fn toggle_single_diff_hunk(&mut self, range: Range<Anchor>, cx: &mut Context<Self>) {
        self.buffer.update(cx, |buffer, cx| {
            buffer.toggle_single_diff_hunk(range, cx);
        })
    }

    pub(super) fn apply_all_diff_hunks(
        &mut self,
        _: &ApplyAllDiffHunks,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.read_only(cx) {
            return;
        }

        let buffers = self.buffer.read(cx).all_buffers();
        for branch_buffer in buffers {
            branch_buffer.update(cx, |branch_buffer, cx| {
                branch_buffer.merge_into_base(Vec::new(), cx);
            });
        }

        if let Some(project) = self.project.clone() {
            self.save(
                SaveOptions {
                    format: true,
                    force_format: false,
                    autosave: false,
                },
                project,
                window,
                cx,
            )
            .detach_and_log_err(cx);
        }
    }

    pub(super) fn apply_selected_diff_hunks(
        &mut self,
        _: &ApplyDiffHunk,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.read_only(cx) {
            return;
        }
        let snapshot = self.snapshot(window, cx);
        let hunks = snapshot.hunks_for_ranges(
            self.selections
                .all(&snapshot.display_snapshot)
                .into_iter()
                .map(|selection| selection.range()),
        );
        let mut ranges_by_buffer = HashMap::default();
        self.transact(window, cx, |editor, _window, cx| {
            for hunk in hunks {
                if let Some(buffer) = editor.buffer.read(cx).buffer(hunk.buffer_id) {
                    ranges_by_buffer
                        .entry(buffer.clone())
                        .or_insert_with(Vec::new)
                        .push(hunk.buffer_range.to_offset(buffer.read(cx)));
                }
            }

            for (buffer, ranges) in ranges_by_buffer {
                buffer.update(cx, |buffer, cx| {
                    buffer.merge_into_base(ranges, cx);
                });
            }
        });

        if let Some(project) = self.project.clone() {
            self.save(
                SaveOptions {
                    format: true,
                    force_format: false,
                    autosave: false,
                },
                project,
                window,
                cx,
            )
            .detach_and_log_err(cx);
        }
    }

    pub(super) fn open_git_blame_commit(
        &mut self,
        _: &OpenGitBlameCommit,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_git_blame_commit_internal(window, cx);
    }

    pub(super) fn toggle_git_blame_inline_internal(
        &mut self,
        user_triggered: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.git_blame_inline_enabled {
            self.git_blame_inline_enabled = false;
            self.show_git_blame_inline = false;
            self.show_git_blame_inline_delay_task.take();
        } else {
            self.git_blame_inline_enabled = true;
            self.start_git_blame_inline(user_triggered, window, cx);
        }

        cx.notify();
    }

    pub(super) fn start_git_blame_inline(
        &mut self,
        user_triggered: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.start_git_blame(user_triggered, window, cx);

        if ProjectSettings::get_global(cx)
            .git
            .inline_blame_delay()
            .is_some()
        {
            self.start_inline_blame_timer(window, cx);
        } else {
            self.show_git_blame_inline = true
        }
    }

    pub(super) fn render_git_blame_gutter(&self, cx: &App) -> bool {
        !self.mode().is_minimap() && self.show_git_blame_gutter && self.has_blame_entries(cx)
    }

    pub(super) fn render_git_blame_inline(&self, window: &Window, cx: &App) -> bool {
        ProjectSettings::get_global(cx).git.inline_blame.location
            == project::project_settings::InlineBlameLocation::Inline
            && self.show_git_blame_inline
            && (self.focus_handle.is_focused(window) || self.inline_blame_popover.is_some())
            && !self.newest_selection_head_on_empty_line(cx)
            && self.has_blame_entries(cx)
    }

    pub(super) fn start_inline_blame_timer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(delay) = ProjectSettings::get_global(cx).git.inline_blame_delay() {
            self.show_git_blame_inline = false;

            self.show_git_blame_inline_delay_task =
                Some(cx.spawn_in(window, async move |this, cx| {
                    cx.background_executor().timer(delay).await;

                    this.update(cx, |this, cx| {
                        this.show_git_blame_inline = true;
                        cx.notify();
                    })
                    .log_err();
                }));
        }
    }

    pub(super) fn show_blame_popover(
        &mut self,
        buffer: BufferId,
        blame_entry: &BlameEntry,
        position: gpui::Point<Pixels>,
        ignore_timeout: bool,
        cx: &mut Context<Self>,
    ) {
        if let Some(state) = &mut self.inline_blame_popover {
            state.hide_task.take();
        } else {
            let blame_popover_delay = EditorSettings::get_global(cx).hover_popover_delay.0;
            let blame_entry = blame_entry.clone();
            let show_task = cx.spawn(async move |editor, cx| {
                if !ignore_timeout {
                    cx.background_executor()
                        .timer(std::time::Duration::from_millis(blame_popover_delay))
                        .await;
                }
                editor
                    .update(cx, |editor, cx| {
                        editor.inline_blame_popover_show_task.take();
                        let Some(blame) = editor.blame.as_ref() else {
                            return;
                        };
                        let blame = blame.read(cx);
                        let details = blame.details_for_entry(buffer, &blame_entry);
                        let markdown = cx.new(|cx| {
                            Markdown::new(
                                details
                                    .as_ref()
                                    .map(|message| message.message.clone())
                                    .unwrap_or_default(),
                                None,
                                None,
                                cx,
                            )
                        });
                        editor.inline_blame_popover = Some(InlineBlamePopover {
                            position,
                            hide_task: None,
                            popover_bounds: None,
                            popover_state: InlineBlamePopoverState {
                                scroll_handle: ScrollHandle::new(),
                                commit_message: details,
                                markdown,
                            },
                            keyboard_grace: ignore_timeout,
                        });
                        cx.notify();
                    })
                    .ok();
            });
            self.inline_blame_popover_show_task = Some(show_task);
        }
    }

    pub(super) fn go_to_prev_hunk(
        &mut self,
        _: &GoToPreviousHunk,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let snapshot = self.snapshot(window, cx);
        let selection = self.selections.newest::<Point>(&snapshot.display_snapshot);
        self.go_to_hunk_before_or_after_position(
            &snapshot,
            selection.head(),
            Direction::Prev,
            true,
            window,
            cx,
        );
    }

    pub fn stage_or_unstage_diff_hunks(
        &mut self,
        stage: bool,
        ranges: Vec<Range<Anchor>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let task = self.save_buffers_for_ranges_if_needed(&ranges, cx);
        cx.spawn_in(window, async move |this, cx| {
            task.await?;
            this.update_in(cx, |this, window, cx| {
                let snapshot = this.buffer.read(cx).snapshot(cx);
                let hunks = this.diff_hunks_in_ranges(&ranges, &snapshot).collect();
                this.apply_stage_or_unstage(stage, hunks, window, cx);
            })
        })
        .detach_and_log_err(cx);
    }

    pub fn restore_diff_hunks_in_ranges(
        &mut self,
        ranges: Vec<Range<Anchor>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let snapshot = self.buffer.read(cx).snapshot(cx);
        let hunks = self.diff_hunks_in_ranges(&ranges, &snapshot).collect();
        self.apply_restore(hunks, window, cx);
    }

    fn toggle_diff_hunks_in_ranges(
        &mut self,
        ranges: Vec<Range<Anchor>>,
        cx: &mut Context<Editor>,
    ) {
        self.buffer.update(cx, |buffer, cx| {
            let expand = !buffer.has_expanded_diff_hunks_in_ranges(&ranges, cx);
            buffer.expand_or_collapse_diff_hunks(ranges, expand, cx);
        })
    }

    pub fn set_blame(
        &mut self,
        blame: Entity<GitBlame>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.blame_subscription = Some(cx.observe_in(&blame, window, |_, _, _, cx| cx.notify()));
        self.blame = Some(blame);
        self.show_git_blame_gutter = true;
        cx.notify();
    }

    fn start_git_blame(
        &mut self,
        user_triggered: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self
            .blame
            .as_ref()
            .is_some_and(|blame| blame.read(cx).is_static())
        {
            return;
        }
        if let Some(project) = self.project() {
            if let Some(buffer) = self.buffer().read(cx).as_singleton()
                && buffer.read(cx).file().is_none_or(|file| {
                    matches!(file.disk_state(), language::DiskState::Historic { .. })
                })
            {
                return;
            }

            let focused = self.focus_handle(cx).contains_focused(window, cx);

            let project = project.clone();
            let blame = cx
                .new(|cx| GitBlame::new(self.buffer.clone(), project, user_triggered, focused, cx));
            self.blame_subscription =
                Some(cx.observe_in(&blame, window, |_, _, _, cx| cx.notify()));
            self.blame = Some(blame);
        }
    }

    fn restore_hunks_in_ranges(
        &mut self,
        ranges: Vec<Range<Point>>,
        window: &mut Window,
        cx: &mut Context<Editor>,
    ) {
        let hunks = self.snapshot(window, cx).hunks_for_ranges(ranges);
        self.apply_restore(hunks, window, cx);
    }

    fn save_buffers_for_ranges_if_needed(
        &mut self,
        ranges: &[Range<Anchor>],
        cx: &mut Context<Editor>,
    ) -> Task<Result<()>> {
        let multibuffer = self.buffer.read(cx);
        let snapshot = multibuffer.read(cx);
        let buffer_ids: HashSet<_> = ranges
            .iter()
            .flat_map(|range| snapshot.buffer_ids_for_range(range.clone()))
            .collect();
        drop(snapshot);

        let mut buffers = HashSet::default();
        for buffer_id in buffer_ids {
            if let Some(buffer_entity) = multibuffer.buffer(buffer_id) {
                let buffer = buffer_entity.read(cx);
                if buffer.file().is_some_and(|file| file.disk_state().exists()) && buffer.is_dirty()
                {
                    buffers.insert(buffer_entity);
                }
            }
        }

        if let Some(project) = &self.project {
            project.update(cx, |project, cx| project.save_buffers(buffers, cx))
        } else {
            Task::ready(Ok(()))
        }
    }

    fn do_stage_or_unstage_and_next(
        &mut self,
        stage: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let ranges = self.selections.disjoint_anchor_ranges().collect::<Vec<_>>();

        if ranges.iter().any(|range| range.start != range.end) {
            self.stage_or_unstage_diff_hunks(stage, ranges, window, cx);
            return;
        }

        self.stage_or_unstage_diff_hunks(stage, ranges, window, cx);

        let all_diff_hunks_expanded = self.buffer().read(cx).all_diff_hunks_expanded();
        let wrap_around = !all_diff_hunks_expanded;
        let snapshot = self.snapshot(window, cx);
        let position = self
            .selections
            .newest::<Point>(&snapshot.display_snapshot)
            .head();

        self.go_to_hunk_before_or_after_position(
            &snapshot,
            position,
            Direction::Next,
            wrap_around,
            window,
            cx,
        );
    }

    fn open_git_blame_commit_internal(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<()> {
        let (blame_entry, repo) = self.blame_entry_at_cursor(window, cx)?;
        let renderer = cx.global::<GlobalBlameRenderer>().0.clone();
        let workspace = self.workspace()?.downgrade();
        renderer.open_blame_commit(blame_entry, repo, workspace, window, cx);
        None
    }

    fn blame_entry_at_cursor(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<(BlameEntry, Entity<Repository>)> {
        let blame = self.blame.clone()?;
        let snapshot = self.snapshot(window, cx);
        let cursor = self
            .selections
            .newest::<Point>(&snapshot.display_snapshot)
            .head();
        let (buffer, point) = snapshot.buffer_snapshot().point_to_buffer_point(cursor)?;
        let (_, blame_entry) = blame
            .update(cx, |blame, cx| {
                blame
                    .blame_for_rows(
                        &[RowInfo {
                            buffer_id: Some(buffer.remote_id()),
                            buffer_row: Some(point.row),
                            ..Default::default()
                        }],
                        cx,
                    )
                    .next()
            })
            .flatten()?;
        let repository = blame.read(cx).repository(cx, buffer.remote_id())?;
        Some((blame_entry, repository))
    }

    pub(crate) fn blame_revision_target(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<(RepoPath, Oid, Entity<Repository>)> {
        let (blame_entry, repository) = self.blame_entry_at_cursor(window, cx)?;
        let highlighted_sha = self
            .blame
            .as_ref()
            .and_then(|blame| blame.read(cx).highlighted_sha());
        let (revision, path) = blame_entry.revision_target(highlighted_sha)?;
        Some((path, revision, repository))
    }

    pub(crate) fn blame_previous_revision_target(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<(RepoPath, Oid, Entity<Repository>)> {
        let (blame_entry, repository) = self.blame_entry_at_cursor(window, cx)?;
        let (revision, path) = blame_entry.previous_revision_target()?;
        Some((path, revision, repository))
    }

    pub(super) fn blame_revision(
        &mut self,
        _: &BlameRevision,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((path, revision, repository)) = self.blame_revision_target(window, cx) else {
            return;
        };
        self.open_blame_revision(path, revision, repository, window, cx);
    }

    pub(super) fn blame_previous_revision(
        &mut self,
        _: &BlamePreviousRevision,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((path, revision, repository)) = self.blame_previous_revision_target(window, cx)
        else {
            return;
        };
        self.open_blame_revision(path, revision, repository, window, cx);
    }

    fn open_blame_revision(
        &mut self,
        path: RepoPath,
        revision: Oid,
        repository: Entity<Repository>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(workspace) = self.workspace() else {
            return;
        };
        let renderer = cx.global::<GlobalBlameRenderer>().0.clone();
        renderer.open_blame_revision(
            path,
            revision,
            repository,
            workspace.downgrade(),
            window,
            cx,
        );
    }

    fn has_blame_entries(&self, cx: &App) -> bool {
        self.blame()
            .is_some_and(|blame| blame.read(cx).has_generated_entries())
    }

    fn newest_selection_head_on_empty_line(&self, cx: &App) -> bool {
        let cursor_anchor = self.selections.newest_anchor().head();

        let snapshot = self.buffer.read(cx).snapshot(cx);
        let buffer_row = MultiBufferRow(cursor_anchor.to_point(&snapshot).row);

        snapshot.line_len(buffer_row) == 0
    }
    fn hunk_after_position(
        &mut self,
        snapshot: &EditorSnapshot,
        position: Point,
        wrap_around: bool,
    ) -> Option<MultiBufferDiffHunk> {
        let result = snapshot
            .buffer_snapshot()
            .diff_hunks_in_range(position..snapshot.buffer_snapshot().max_point())
            .find(|hunk| hunk.row_range.start.0 > position.row);

        if wrap_around {
            result.or_else(|| {
                snapshot
                    .buffer_snapshot()
                    .diff_hunks_in_range(Point::zero()..position)
                    .find(|hunk| hunk.row_range.end.0 < position.row)
            })
        } else {
            result
        }
    }

    fn hunk_before_position(
        &mut self,
        snapshot: &EditorSnapshot,
        position: Point,
        wrap_around: bool,
    ) -> Option<MultiBufferRow> {
        let result = snapshot.buffer_snapshot().diff_hunk_before(position);

        if wrap_around {
            result.or_else(|| snapshot.buffer_snapshot().diff_hunk_before(Point::MAX))
        } else {
            result
        }
    }

    fn render_diff_review_overlay(
        editor_handle: &WeakEntity<Editor>,
        cx: &mut BlockContext,
    ) -> AnyElement {
        fn format_line_ranges(ranges: &[(u32, u32)]) -> Option<String> {
            if ranges.is_empty() {
                return None;
            }
            let formatted: Vec<String> = ranges
                .iter()
                .map(|(start, end)| {
                    let start_line = start + 1;
                    let end_line = end + 1;
                    if start_line == end_line {
                        format!("Line {start_line}")
                    } else {
                        format!("Lines {start_line}-{end_line}")
                    }
                })
                .collect();
            // Don't show label for single line in single excerpt
            if ranges.len() == 1 && ranges[0].0 == ranges[0].1 {
                return None;
            }
            Some(formatted.join(" ⋯ "))
        }

        let Some(editor) = editor_handle.upgrade() else {
            return gpui::Empty.into_any_element();
        };
        let BlockId::Custom(block_id) = cx.block_id else {
            return gpui::Empty.into_any_element();
        };
        let editor = editor.read(cx);
        let Some(overlay) = editor
            .diff_review_overlays
            .iter()
            .find(|overlay| overlay.block_id == block_id)
        else {
            return gpui::Empty.into_any_element();
        };
        let snapshot = editor.buffer.read(cx).snapshot(cx);
        let start = overlay.anchor_range.start.to_point(&snapshot);
        let end = overlay.anchor_range.end.to_point(&snapshot);
        let ranges: Vec<_> = snapshot
            .range_to_buffer_ranges(start..end)
            .iter()
            .map(|(buffer, range, _)| {
                (
                    buffer.offset_to_point(range.start.0).row,
                    buffer.offset_to_point(range.end.0).row,
                )
            })
            .collect();

        let theme = cx.theme();
        let colors = theme.colors();
        let avatar_size = px(16.);
        let action_icon_size = IconSize::XSmall;

        let root = v_flex()
            .id(("diff-review-overlay", overlay.prompt_editor.entity_id()))
            .w_full()
            .bg(colors.editor_background)
            .border_b_1()
            .border_color(colors.border)
            .px_2()
            .pb_1p5()
            .gap_1()
            .when_some(format_line_ranges(&ranges), |element, label| {
                element.child(
                    h_flex()
                        .w_full()
                        .px_2()
                        .child(Label::new(label).size(LabelSize::Small).color(Color::Muted)),
                )
            });

        let Some(comment_id) = overlay.comment_id else {
            let close = editor_handle.clone();
            let submit = editor_handle.clone();
            let prompt_editor = overlay.prompt_editor.clone();
            let user_avatar_uri = overlay.user_avatar_uri.clone();
            return root
                .child(
                    h_flex()
                        .w_full()
                        .items_center()
                        .gap_1()
                        .px_2()
                        .py_1()
                        .rounded_md()
                        .bg(colors.surface_background)
                        .child(
                            div()
                                .size(avatar_size)
                                .flex_shrink_0()
                                .rounded_full()
                                .overflow_hidden()
                                .child(if let Some(ref avatar_uri) = user_avatar_uri {
                                    Avatar::new(avatar_uri.clone())
                                        .size(avatar_size)
                                        .into_any_element()
                                } else {
                                    Icon::new(IconName::Person)
                                        .size(IconSize::XSmall)
                                        .color(ui::Color::Muted)
                                        .into_any_element()
                                }),
                        )
                        .child(
                            div()
                                .flex_1()
                                .border_1()
                                .border_color(colors.border)
                                .rounded_md()
                                .bg(colors.editor_background)
                                .px_2()
                                .py_0p5()
                                .child(prompt_editor.clone()),
                        )
                        .child(
                            h_flex()
                                .flex_shrink_0()
                                .gap_0p5()
                                .child(
                                    IconButton::new("diff-review-close", IconName::Close)
                                        .icon_color(ui::Color::Muted)
                                        .icon_size(action_icon_size)
                                        .aria_label("Close review composer")
                                        .tooltip(Tooltip::text("Close review composer"))
                                        .on_click(move |_, window, cx| {
                                            close
                                                .update(cx, |editor, cx| {
                                                    editor.close_diff_review_overlay(
                                                        block_id, window, cx,
                                                    );
                                                })
                                                .log_err();
                                        }),
                                )
                                .child(
                                    IconButton::new("diff-review-add", IconName::Return)
                                        .icon_color(ui::Color::Muted)
                                        .icon_size(action_icon_size)
                                        .aria_label("Add review comment")
                                        .tooltip(Tooltip::text("Add review comment"))
                                        .on_click(move |_, window, cx| {
                                            submit
                                                .update(cx, |editor, cx| {
                                                    editor.submit_diff_review_overlay(
                                                        block_id, window, cx,
                                                    );
                                                })
                                                .log_err();
                                        }),
                                ),
                        ),
                )
                .into_any_element();
        };

        let Some(comment) = editor.review_comment(comment_id) else {
            return gpui::Empty.into_any_element();
        };
        let editor_handle = editor_handle.clone();
        if !overlay.comments_expanded {
            let preview = comment
                .comment
                .lines()
                .next()
                .unwrap_or_default()
                .to_string();
            return root
                .child(
                    h_flex().min_w_0().child(
                        Button::new("diff-review-expand", preview)
                            .start_icon(Icon::new(IconName::ChevronRight).size(IconSize::Small))
                            .truncate(true)
                            .on_click(move |_, window, cx| {
                                editor_handle
                                    .update(cx, |editor, cx| {
                                        editor.toggle_diff_review_overlay(block_id, window, cx);
                                    })
                                    .log_err();
                            }),
                    ),
                )
                .into_any_element();
        }

        let close = IconButton::new("diff-review-collapse", IconName::Close)
            .icon_color(ui::Color::Muted)
            .icon_size(action_icon_size)
            .aria_label("Collapse review comment")
            .tooltip(Tooltip::text("Collapse review comment"))
            .on_click(move |_, window, cx| {
                editor_handle
                    .update(cx, |editor, cx| {
                        editor.close_diff_review_overlay(block_id, window, cx);
                    })
                    .log_err();
            });
        root.child(Self::render_comment_row(
            comment.clone(),
            overlay
                .inline_edit
                .as_ref()
                .map(|(editor, _)| editor.clone()),
            overlay.user_avatar_uri.clone(),
            avatar_size,
            action_icon_size,
            colors,
            close,
        ))
        .into_any_element()
    }

    fn render_comment_row(
        comment: StoredReviewComment,
        inline_editor: Option<Entity<Editor>>,
        user_avatar_uri: Option<SharedUri>,
        avatar_size: Pixels,
        action_icon_size: IconSize,
        colors: &theme::ThemeColors,
        close: IconButton,
    ) -> impl IntoElement {
        let comment_id = comment.id;
        let is_editing = inline_editor.is_some();
        let status = comment.status;
        let status_color = match status {
            ReviewCommentStatus::Draft
            | ReviewCommentStatus::Queued
            | ReviewCommentStatus::InReview => ui::Color::Muted,
            ReviewCommentStatus::Sent | ReviewCommentStatus::Resolved => ui::Color::Success,
            ReviewCommentStatus::Outdated => ui::Color::Warning,
        };
        let group_id = SharedString::from(format!("diff-review-comment-{comment_id}"));

        h_flex()
            .id(SharedString::from(format!(
                "diff-review-comment-row-{comment_id}"
            )))
            .w_full()
            .items_start()
            .gap_1()
            .px_2()
            .py_1()
            .rounded_md()
            .bg(colors.surface_background)
            .group(group_id.clone())
            .child(
                div()
                    .size(avatar_size)
                    .flex_shrink_0()
                    .mt_0p5()
                    .rounded_full()
                    .overflow_hidden()
                    .child(if let Some(ref avatar_uri) = user_avatar_uri {
                        Avatar::new(avatar_uri.clone())
                            .size(avatar_size)
                            .into_any_element()
                    } else {
                        Icon::new(IconName::Person)
                            .size(IconSize::XSmall)
                            .color(ui::Color::Muted)
                            .into_any_element()
                    }),
            )
            .child(
                v_flex()
                    .flex_1()
                    .gap_0p5()
                    .min_w_0()
                    .child(
                        h_flex()
                            .w_full()
                            .items_center()
                            .gap_1()
                            .child(Label::new("You").size(LabelSize::Small).color(Color::Muted))
                            .child(
                                h_flex()
                                    .items_center()
                                    .gap_0p5()
                                    .child(
                                        Icon::new(match status {
                                            ReviewCommentStatus::Sent
                                            | ReviewCommentStatus::Resolved => IconName::Check,
                                            ReviewCommentStatus::Outdated => IconName::Warning,
                                            _ => IconName::FileDiff,
                                        })
                                        .size(IconSize::XSmall)
                                        .color(status_color),
                                    )
                                    .child(
                                        Label::new(status.label())
                                            .size(LabelSize::Small)
                                            .color(status_color),
                                    ),
                            ),
                    )
                    .child(if let Some(editor) = inline_editor {
                        div()
                            .w_full()
                            .border_1()
                            .border_color(colors.border)
                            .rounded_md()
                            .bg(colors.editor_background)
                            .px_2()
                            .py_0p5()
                            .child(editor)
                            .into_any_element()
                    } else {
                        div()
                            .w_full()
                            .text_sm()
                            .text_color(colors.text)
                            .child(comment.comment)
                            .into_any_element()
                    })
                    .when_some(comment.agent_reply, |this, (agent, reply)| {
                        this.child(
                            v_flex()
                                .min_w_0()
                                .mt_1()
                                .pl_2()
                                .border_l_1()
                                .border_color(colors.border)
                                .child(Label::new(agent).size(LabelSize::Small).color(Color::Muted))
                                .child(
                                    div()
                                        .w_full()
                                        .text_sm()
                                        .text_color(colors.text)
                                        .child(reply),
                                ),
                        )
                    }),
            )
            .child(if is_editing {
                h_flex()
                    .gap_0p5()
                    .child(
                        IconButton::new(
                            format!("diff-review-cancel-edit-{comment_id}"),
                            IconName::Close,
                        )
                        .icon_color(ui::Color::Muted)
                        .icon_size(action_icon_size)
                        .tooltip(Tooltip::text("Cancel"))
                        .on_click(move |_, window, cx| {
                            window.dispatch_action(
                                Box::new(crate::actions::CancelEditReviewComment {
                                    id: comment_id,
                                }),
                                cx,
                            );
                        }),
                    )
                    .child(
                        IconButton::new(
                            format!("diff-review-confirm-edit-{comment_id}"),
                            IconName::Return,
                        )
                        .icon_color(ui::Color::Muted)
                        .icon_size(action_icon_size)
                        .tooltip(Tooltip::text("Confirm"))
                        .on_click(move |_, window, cx| {
                            window.dispatch_action(
                                Box::new(crate::actions::ConfirmEditReviewComment {
                                    id: comment_id,
                                }),
                                cx,
                            );
                        }),
                    )
                    .into_any_element()
            } else {
                h_flex()
                    .gap_0p5()
                    .visible_on_hover(group_id)
                    .when(status.is_sendable(), |this| {
                        this.child(
                            IconButton::new(
                                format!("diff-review-edit-{comment_id}"),
                                IconName::Pencil,
                            )
                            .icon_color(ui::Color::Muted)
                            .icon_size(action_icon_size)
                            .tooltip(Tooltip::text("Edit comment"))
                            .on_click(move |_, window, cx| {
                                window.dispatch_action(
                                    Box::new(crate::actions::EditReviewComment { id: comment_id }),
                                    cx,
                                );
                            }),
                        )
                    })
                    .child(
                        IconButton::new(
                            format!("diff-review-delete-{comment_id}"),
                            IconName::Trash,
                        )
                        .icon_color(ui::Color::Muted)
                        .icon_size(action_icon_size)
                        .tooltip(Tooltip::text("Delete comment"))
                        .on_click(move |_, window, cx| {
                            window.dispatch_action(
                                Box::new(crate::actions::DeleteReviewComment { id: comment_id }),
                                cx,
                            );
                        }),
                    )
                    .into_any_element()
            })
            .child(close)
    }

    fn get_permalink_to_line(&self, cx: &mut Context<Self>) -> Task<Result<url::Url>> {
        let buffer_and_selection = maybe!({
            let selection = self.selections.newest::<Point>(&self.display_snapshot(cx));
            let selection_range = selection.range();

            let multi_buffer = self.buffer().read(cx);
            let multi_buffer_snapshot = multi_buffer.snapshot(cx);
            let buffer_ranges = multi_buffer_snapshot
                .range_to_buffer_ranges(selection_range.start..selection_range.end);

            let (buffer_snapshot, range, _) = if selection.reversed {
                buffer_ranges.first()
            } else {
                buffer_ranges.last()
            }?;

            let buffer_range = range.to_point(buffer_snapshot);
            let buffer = multi_buffer.buffer(buffer_snapshot.remote_id())?;

            let Some(buffer_diff) = multi_buffer.diff_for(buffer_snapshot.remote_id()) else {
                return Some((buffer, buffer_range.start.row..buffer_range.end.row));
            };

            let buffer_diff_snapshot = buffer_diff.read(cx).snapshot(cx);
            let start = buffer_diff_snapshot
                .buffer_point_to_base_text_point(buffer_range.start, &buffer_snapshot);
            let end = buffer_diff_snapshot
                .buffer_point_to_base_text_point(buffer_range.end, &buffer_snapshot);

            Some((buffer, start.row..end.row))
        });

        let Some((buffer, selection)) = buffer_and_selection else {
            return Task::ready(Err(anyhow!("failed to determine buffer and selection")));
        };

        let Some(project) = self.project() else {
            return Task::ready(Err(anyhow!("editor does not have project")));
        };

        project.update(cx, |project, cx| {
            project.get_permalink_to_line(&buffer, selection, cx)
        })
    }
}

#[cfg(test)]
impl Editor {
    /// Returns the line range for the first diff review overlay, if one is active.
    /// Returns (start_row, end_row) as physical line numbers in the underlying file.
    pub(super) fn diff_review_line_range(&self, cx: &App) -> Option<(u32, u32)> {
        let overlay = self.diff_review_overlays.first()?;
        let snapshot = self.buffer.read(cx).snapshot(cx);
        let start_point = overlay.anchor_range.start.to_point(&snapshot);
        let end_point = overlay.anchor_range.end.to_point(&snapshot);
        let start_row = snapshot
            .point_to_buffer_point(start_point)
            .map(|(_, p)| p.row)
            .unwrap_or(start_point.row);
        let end_row = snapshot
            .point_to_buffer_point(end_point)
            .map(|(_, p)| p.row)
            .unwrap_or(end_point.row);
        Some((start_row, end_row))
    }

    /// Takes all stored comments from all hunks, clearing the storage.
    /// Returns a Vec of (hunk_key, comments) pairs.
    pub(super) fn take_all_review_comments(
        &mut self,
        cx: &mut Context<Self>,
    ) -> Vec<(DiffHunkKey, Vec<StoredReviewComment>)> {
        // Dismiss all overlays when taking comments (e.g., when sending to agent)
        self.dismiss_all_diff_review_overlays(cx);
        let comments = std::mem::take(&mut self.stored_review_comments);
        // Reset the ID counter since all comments have been taken
        self.next_review_comment_id = 0;
        cx.emit(EditorEvent::ReviewCommentsChanged { total_count: 0 });
        cx.notify();
        comments
    }
}

impl EditorSnapshot {
    pub(super) fn display_diff_hunks_for_rows<'a>(
        &'a self,
        display_rows: Range<DisplayRow>,
        folded_buffers: &'a HashSet<BufferId>,
    ) -> impl 'a + Iterator<Item = DisplayDiffHunk> {
        let buffer_start = DisplayPoint::new(display_rows.start, 0).to_point(self);
        let buffer_end = DisplayPoint::new(display_rows.end, 0).to_point(self);

        self.buffer_snapshot()
            .diff_hunks_in_range(buffer_start..buffer_end)
            .filter_map(|hunk| {
                if folded_buffers.contains(&hunk.buffer_id)
                    || (hunk.row_range.is_empty() && self.buffer.all_diff_hunks_expanded())
                {
                    return None;
                }

                let hunk_start_point = Point::new(hunk.row_range.start.0, 0);
                let hunk_end_point = if hunk.row_range.end > hunk.row_range.start {
                    let last_row = MultiBufferRow(hunk.row_range.end.0 - 1);
                    let line_len = self.buffer_snapshot().line_len(last_row);
                    Point::new(last_row.0, line_len)
                } else {
                    Point::new(hunk.row_range.end.0, 0)
                };

                let hunk_display_start = self.point_to_display_point(hunk_start_point, Bias::Left);
                let hunk_display_end = self.point_to_display_point(hunk_end_point, Bias::Right);

                let display_hunk = if hunk_display_start.column() != 0 {
                    DisplayDiffHunk::Folded {
                        display_row: hunk_display_start.row(),
                    }
                } else {
                    let mut end_row = hunk_display_end.row();
                    if hunk.row_range.end > hunk.row_range.start || hunk_display_end.column() > 0 {
                        end_row.0 += 1;
                    }
                    let is_created_file = hunk.is_created_file();
                    let multi_buffer_range = hunk.multi_buffer_range.clone();

                    DisplayDiffHunk::Unfolded {
                        status: hunk.status(),
                        diff_base_byte_range: hunk.diff_base_byte_range.start.0
                            ..hunk.diff_base_byte_range.end.0,
                        word_diffs: hunk.word_diffs,
                        display_row_range: hunk_display_start.row()..end_row,
                        multi_buffer_range,
                        is_created_file,
                    }
                };

                Some(display_hunk)
            })
    }

    fn hunks_for_ranges(
        &self,
        ranges: impl IntoIterator<Item = Range<Point>>,
    ) -> Vec<MultiBufferDiffHunk> {
        let mut hunks = Vec::new();
        let mut processed_buffer_rows: HashMap<BufferId, HashSet<Range<text::Anchor>>> =
            HashMap::default();
        for query_range in ranges {
            let query_rows =
                MultiBufferRow(query_range.start.row)..MultiBufferRow(query_range.end.row + 1);
            for hunk in self.buffer_snapshot().diff_hunks_in_range(
                Point::new(query_rows.start.0, 0)..Point::new(query_rows.end.0, 0),
            ) {
                // Include deleted hunks that are adjacent to the query range, because
                // otherwise they would be missed.
                let mut intersects_range = hunk.row_range.overlaps(&query_rows);
                if hunk.status().is_deleted() {
                    intersects_range |= hunk.row_range.start == query_rows.end;
                    intersects_range |= hunk.row_range.end == query_rows.start;
                }
                if intersects_range {
                    if !processed_buffer_rows
                        .entry(hunk.buffer_id)
                        .or_default()
                        .insert(hunk.buffer_range.start..hunk.buffer_range.end)
                    {
                        continue;
                    }
                    hunks.push(hunk);
                }
            }
        }

        hunks
    }
}

pub fn set_blame_renderer(renderer: impl BlameRenderer + 'static, cx: &mut App) {
    cx.set_global(GlobalBlameRenderer(Arc::new(renderer)));
}

pub fn render_diff_hunk_controls(
    row: u32,
    status: &DiffHunkStatus,
    hunk_range: Range<Anchor>,
    is_created_file: bool,
    line_height: Pixels,
    editor: &Entity<Editor>,
    _window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let operations = hunk_range
        .start
        .buffer_id()
        .and_then(|buffer_id| editor.read(cx).buffer().read(cx).diff_for(buffer_id))
        .and_then(|diff| diff.read(cx).operations());
    let show_stage_restore = ProjectSettings::get_global(cx)
        .git
        .show_stage_restore_buttons;
    let supports_staging = operations
        .as_ref()
        .is_some_and(|operations| operations.supports_staging());
    let supports_unstaging = operations
        .as_ref()
        .is_some_and(|operations| operations.supports_unstaging());
    let supports_restore = operations
        .as_ref()
        .is_some_and(|operations| operations.supports_restore());
    let show_stage_or_unstage = show_stage_restore
        && ((status.has_secondary_hunk() && supports_staging)
            || (!status.has_secondary_hunk() && supports_unstaging));
    let show_restore = show_stage_restore && supports_restore;
    let show_hunk_navigation = !editor.read(cx).buffer().read(cx).all_diff_hunks_expanded();

    if !show_stage_or_unstage && !show_restore && !show_hunk_navigation {
        return gpui::Empty.into_any_element();
    }

    h_flex()
        .h(line_height)
        .mr_1()
        .gap_1()
        .px_0p5()
        .pb_1()
        .border_x_1()
        .border_b_1()
        .border_color(cx.theme().colors().border_variant)
        .rounded_b_lg()
        .bg(cx.theme().colors().editor_background)
        .gap_1()
        .block_mouse_except_scroll()
        .shadow_md()
        .when(show_stage_or_unstage, |el| {
            el.child(if status.has_secondary_hunk() {
                Button::new(("stage", row as u64), "Stage")
                    .alpha(if status.is_pending() { 0.66 } else { 1.0 })
                    .tooltip({
                        let focus_handle = editor.focus_handle(cx);
                        move |_window, cx| {
                            Tooltip::for_action_in(
                                "Stage Hunk",
                                &::git::ToggleStaged,
                                &focus_handle,
                                cx,
                            )
                        }
                    })
                    .on_click({
                        let editor = editor.clone();
                        move |_event, window, cx| {
                            editor.update(cx, |editor, cx| {
                                editor.stage_or_unstage_diff_hunks(
                                    true,
                                    vec![hunk_range.start..hunk_range.start],
                                    window,
                                    cx,
                                );
                            });
                        }
                    })
            } else {
                Button::new(("unstage", row as u64), "Unstage")
                    .alpha(if status.is_pending() { 0.66 } else { 1.0 })
                    .tooltip({
                        let focus_handle = editor.focus_handle(cx);
                        move |_window, cx| {
                            Tooltip::for_action_in(
                                "Unstage Hunk",
                                &::git::ToggleStaged,
                                &focus_handle,
                                cx,
                            )
                        }
                    })
                    .on_click({
                        let editor = editor.clone();
                        move |_event, window, cx| {
                            editor.update(cx, |editor, cx| {
                                editor.stage_or_unstage_diff_hunks(
                                    false,
                                    vec![hunk_range.start..hunk_range.start],
                                    window,
                                    cx,
                                );
                            });
                        }
                    })
            })
        })
        .when(show_restore, |el| {
            el.child(
                Button::new(("restore", row as u64), "Restore")
                    .tooltip({
                        let focus_handle = editor.focus_handle(cx);
                        move |_window, cx| {
                            Tooltip::for_action_in(
                                "Restore Hunk",
                                &::git::Restore,
                                &focus_handle,
                                cx,
                            )
                        }
                    })
                    .on_click({
                        let editor = editor.clone();
                        move |_event, window, cx| {
                            editor.update(cx, |editor, cx| {
                                let snapshot = editor.snapshot(window, cx);
                                let point = hunk_range.start.to_point(&snapshot.buffer_snapshot());
                                editor.restore_hunks_in_ranges(vec![point..point], window, cx);
                            });
                        }
                    })
                    .disabled(is_created_file),
            )
        })
        .when(show_hunk_navigation, |el| {
            el.child(
                IconButton::new(("next-hunk", row as u64), IconName::ArrowDown)
                    .shape(IconButtonShape::Square)
                    .icon_size(IconSize::Small)
                    // .disabled(!has_multiple_hunks)
                    .tooltip({
                        let focus_handle = editor.focus_handle(cx);
                        move |_window, cx| {
                            Tooltip::for_action_in("Next Hunk", &GoToHunk, &focus_handle, cx)
                        }
                    })
                    .on_click({
                        let editor = editor.clone();
                        move |_event, window, cx| {
                            editor.update(cx, |editor, cx| {
                                let snapshot = editor.snapshot(window, cx);
                                let position = hunk_range.end.to_point(&snapshot.buffer_snapshot());
                                editor.go_to_hunk_before_or_after_position(
                                    &snapshot,
                                    position,
                                    Direction::Next,
                                    true,
                                    window,
                                    cx,
                                );
                                editor.expand_selected_diff_hunks(cx);
                            });
                        }
                    }),
            )
            .child(
                IconButton::new(("prev-hunk", row as u64), IconName::ArrowUp)
                    .shape(IconButtonShape::Square)
                    .icon_size(IconSize::Small)
                    // .disabled(!has_multiple_hunks)
                    .tooltip({
                        let focus_handle = editor.focus_handle(cx);
                        move |_window, cx| {
                            Tooltip::for_action_in(
                                "Previous Hunk",
                                &GoToPreviousHunk,
                                &focus_handle,
                                cx,
                            )
                        }
                    })
                    .on_click({
                        let editor = editor.clone();
                        move |_event, window, cx| {
                            editor.update(cx, |editor, cx| {
                                let snapshot = editor.snapshot(window, cx);
                                let point = hunk_range.start.to_point(&snapshot.buffer_snapshot());
                                editor.go_to_hunk_before_or_after_position(
                                    &snapshot,
                                    point,
                                    Direction::Prev,
                                    true,
                                    window,
                                    cx,
                                );
                                editor.expand_selected_diff_hunks(cx);
                            });
                        }
                    }),
            )
        })
        .into_any_element()
}

impl Editor {
    pub(super) fn update_uncommitted_diff_for_buffer(
        &mut self,
        project: &Entity<Project>,
        buffers: impl IntoIterator<Item = Entity<Buffer>>,
        cx: &mut Context<Self>,
    ) -> Task<()> {
        let mut tasks = Vec::new();
        project.update(cx, |project, cx| {
            let git_store = project.git_store().clone();
            git_store.update(cx, |git_store, cx| {
                for buffer in buffers {
                    if project::File::from_dyn(buffer.read(cx).file()).is_some() {
                        tasks.push(git_store.open_display_diff(buffer, cx));
                    }
                }
            });
        });

        let editor = cx.entity();
        let buffer = self.buffer.clone();
        cx.spawn(async move |_, cx| {
            let diffs = future::join_all(tasks).await;
            if editor.read_with(cx, |editor, _cx| editor.diff_hunk_renderer.is_some()) {
                return;
            }

            buffer.update(cx, |buffer, cx| {
                for diff in diffs.into_iter().flatten() {
                    buffer.add_diff(diff, cx);
                }
            });
        })
    }
}
