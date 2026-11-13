//! Chat belongs to one world: reloading the same world through the lobby's own
//! Play (the real reopen path) starts with an empty chat log and up-arrow history.
use super::*;

#[test]
#[ignore = "needs a GPU adapter holding the block atlas"]
fn game_harness_chat_is_empty_after_the_same_world_is_reloaded() {
    isolate_saves();
    let name = format!("chat-world-{:?}", std::thread::current().id());
    let mut game = HeadlessGame::boot_into_world(&name);
    game.state.chat.push_log(crate::commands::ChatLine::system("said before the reload", 0));
    game.state.chat.push_history("/time set day".to_string());
    assert!(!game.state.chat.log.is_empty());
    assert!(game.state.leave_world(crate::world_exit::SaveChoice::Save, crate::world_exit::ExitTo::Lobby));
    game.reopen_world(&name);
    assert!(
        !game.state.chat.log.iter().any(|l| l.text.contains("said before the reload")),
        "a line from before the reload is still in the chat"
    );
    assert!(game.state.chat.history.is_empty(), "the up-arrow history survived the reload");
}
