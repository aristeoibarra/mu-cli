use crate::error::{json_ok, Result};
use crate::music;

pub fn handle_play(playlist: Option<&str>, track: Option<&str>) -> Result<()> {
    if let Some(name) = track {
        music::play_track(name)?;
    } else {
        music::play_playlist(playlist)?;
    }
    println!("{}", json_ok("Playing in Apple Music"));
    Ok(())
}

pub fn handle_pause() -> Result<()> {
    music::pause()?;
    println!("{}", json_ok("Paused"));
    Ok(())
}

pub fn handle_resume() -> Result<()> {
    music::resume()?;
    println!("{}", json_ok("Resumed"));
    Ok(())
}

pub fn handle_next() -> Result<()> {
    music::next_track()?;
    println!("{}", json_ok("Next track"));
    Ok(())
}

pub fn handle_previous() -> Result<()> {
    music::previous_track()?;
    println!("{}", json_ok("Previous track"));
    Ok(())
}

pub fn handle_stop() -> Result<()> {
    music::stop()?;
    println!("{}", json_ok("Stopped"));
    Ok(())
}
