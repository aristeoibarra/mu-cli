use notify_rust::Notification;

/// Send desktop notification for new podcast episode
pub fn notify_new_episode(
    podcast_title: &str,
    episode_title: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    Notification::new()
        .summary(&format!("🎙️ {}", podcast_title))
        .body(&format!("New episode: {}", episode_title))
        .timeout(5000) // 5 seconds
        .show()?;

    Ok(())
}
