//! Living world Tier 2 S7 — the Site Board window.
//!
//! Right-clicking a Site Board opens a small window with its job list: a tick
//! box and the job's words for each task, a button to take it off, and an
//! "Add a job" picker listing the twelve jobs (switched off at eight tasks).
//! There is no free text anywhere (call T9): a job is picked, never typed.
//!
//! Pure render: it changes nothing. The caller reads the outcome and applies the
//! op, a local seat through `site_board::use_board` and a joiner by asking the
//! authority (`ItemAction::SiteBoard`); the list it draws is the world's copy
//! (a joiner's is the authority's, shown by a block view).

use crate::site_board::{Job, Op, SiteBoard, MAX_TASKS};

/// What the player did this frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SiteBoardUiOutcome {
    InProgress,
    Closed,
    /// Ask for this op on the board.
    Op(Op),
}

/// The words under the title.
pub const HELP_LINE: &str = "Pick the jobs your crew is on. Anyone can tick one off.";
/// Shown instead of the picker when the board holds [`MAX_TASKS`].
pub const FULL_LINE: &str = "The board is full. Take a job off to add another.";
/// Shown when the board holds nothing yet.
pub const EMPTY_LINE: &str = "Nothing on the board yet.";

/// Title for a board.
pub const TITLE: &str = "Site Board";

/// Draw the window for `board`. `Closed` on Done or the window's close button.
pub fn draw_site_board_ui(ctx: &egui::Context, player_index: usize, board: &SiteBoard) -> SiteBoardUiOutcome {
    let mut outcome = SiteBoardUiOutcome::InProgress;
    let mut window_open = true;

    egui::Window::new(TITLE)
        .id(egui::Id::new(("site_board_window", player_index)))
        .open(&mut window_open)
        .resizable(false)
        .collapsible(false)
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, -40.0))
        .show(ctx, |ui| {
            ui.set_min_width(300.0);
            ui.label(egui::RichText::new(HELP_LINE).size(12.0).italics().weak());
            ui.add_space(6.0);

            if board.tasks.is_empty() {
                ui.label(egui::RichText::new(EMPTY_LINE).weak());
            }
            for (i, task) in board.tasks.iter().enumerate() {
                ui.horizontal(|ui| {
                    let mut done = task.done;
                    if ui.checkbox(&mut done, task.job.label()).changed() {
                        outcome = SiteBoardUiOutcome::Op(Op::toggle(i, task.job));
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button("Remove").clicked() {
                            outcome = SiteBoardUiOutcome::Op(Op::remove(i, task.job));
                        }
                    });
                });
            }

            ui.add_space(8.0);
            ui.separator();
            if board.tasks.len() >= MAX_TASKS {
                ui.label(egui::RichText::new(FULL_LINE).weak());
            } else {
                ui.collapsing("Add a job", |ui| {
                    ui.horizontal_wrapped(|ui| {
                        for job in Job::ALL {
                            if ui.button(job.label()).clicked() {
                                outcome = SiteBoardUiOutcome::Op(Op::add(job));
                            }
                        }
                    });
                });
            }

            ui.add_space(6.0);
            if ui.button("Done").clicked() {
                outcome = SiteBoardUiOutcome::Closed;
            }
        });

    if !window_open {
        return SiteBoardUiOutcome::Closed;
    }
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Run one frame of the window and return what it said.
    fn frame(board: &SiteBoard) -> SiteBoardUiOutcome {
        let ctx = egui::Context::default();
        let mut out = SiteBoardUiOutcome::InProgress;
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            out = draw_site_board_ui(ctx, 0, board);
        });
        out
    }

    #[test]
    fn an_untouched_window_asks_for_nothing() {
        let mut board = SiteBoard::default();
        assert_eq!(frame(&board), SiteBoardUiOutcome::InProgress);
        for job in Job::ALL.iter().take(MAX_TASKS) {
            crate::site_board::apply(&mut board, Op::add(*job)).unwrap();
        }
        assert_eq!(frame(&board), SiteBoardUiOutcome::InProgress, "a full board draws too");
    }

    #[test]
    fn every_job_has_words_the_picker_can_show() {
        for job in Job::ALL {
            assert!(!job.label().is_empty());
            assert!(!job.label().contains("genesis:"));
        }
    }
}
