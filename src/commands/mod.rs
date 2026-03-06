mod add;
pub mod favorites;
mod library;
mod playback;
pub mod plays;
pub mod playlist;

pub use add::handle_add;
pub use favorites::handle_fav_action;
pub use library::{handle_info, handle_list, handle_migrate, handle_reimport, handle_remove, handle_status, handle_sync};
pub use playback::{handle_next, handle_pause, handle_play, handle_previous, handle_resume, handle_stop};
pub use plays::handle_plays_action;
pub use playlist::handle_playlist_action;
