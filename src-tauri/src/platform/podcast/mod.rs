//! Podcast playback support.

use serde::{Deserialize, Serialize};

/// Podcast episode info
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PodcastEpisode {
    pub title: String,
    pub show_name: String,
    pub artwork_url: Option<String>,
    pub audio_url: String,
    pub duration: i32,
    pub progress: i32,
    pub is_downloaded: bool,
    pub local_path: Option<String>,
}

/// Playback state
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlaybackState {
    pub is_playing: bool,
    pub current_time: f64,
    pub duration: f64,
    pub playback_rate: f32,
    pub volume: f32,
}

impl Default for PlaybackState {
    fn default() -> Self {
        Self {
            is_playing: false,
            current_time: 0.0,
            duration: 0.0,
            playback_rate: 1.0,
            volume: 1.0,
        }
    }
}

/// Playback queue
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PlaybackQueue {
    pub episodes: Vec<PodcastEpisode>,
    pub current_index: Option<usize>,
}

impl PlaybackQueue {
    /// Add episode to queue
    pub fn add(&mut self, episode: PodcastEpisode) {
        self.episodes.push(episode);
    }

    /// Get current episode
    pub fn current(&self) -> Option<&PodcastEpisode> {
        self.current_index.and_then(|i| self.episodes.get(i))
    }

    /// Move to next episode
    pub fn next(&mut self) -> Option<&PodcastEpisode> {
        if let Some(index) = self.current_index {
            if index + 1 < self.episodes.len() {
                self.current_index = Some(index + 1);
                return self.episodes.get(index + 1);
            }
        }
        None
    }

    /// Move to previous episode
    pub fn previous(&mut self) -> Option<&PodcastEpisode> {
        if let Some(index) = self.current_index {
            if index > 0 {
                self.current_index = Some(index - 1);
                return self.episodes.get(index - 1);
            }
        }
        None
    }

    /// Clear queue
    pub fn clear(&mut self) {
        self.episodes.clear();
        self.current_index = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_episode(title: &str) -> PodcastEpisode {
        PodcastEpisode {
            title: title.to_string(),
            show_name: "Test Show".to_string(),
            artwork_url: None,
            audio_url: "https://example.com/episode.mp3".to_string(),
            duration: 3600,
            progress: 0,
            is_downloaded: false,
            local_path: None,
        }
    }

    #[test]
    fn test_playback_queue() {
        let mut queue = PlaybackQueue::default();

        queue.add(make_episode("Episode 1"));
        queue.add(make_episode("Episode 2"));
        queue.add(make_episode("Episode 3"));

        assert_eq!(queue.episodes.len(), 3);
        assert!(queue.current().is_none());

        queue.current_index = Some(0);
        assert_eq!(queue.current().unwrap().title, "Episode 1");

        queue.next();
        assert_eq!(queue.current().unwrap().title, "Episode 2");

        queue.previous();
        assert_eq!(queue.current().unwrap().title, "Episode 1");
    }

    #[test]
    fn test_playback_queue_bounds() {
        let mut queue = PlaybackQueue::default();
        queue.add(make_episode("Only Episode"));
        queue.current_index = Some(0);

        // Can't go previous at start
        assert!(queue.previous().is_none());
        assert_eq!(queue.current_index, Some(0));

        // Can't go next at end
        assert!(queue.next().is_none());
        assert_eq!(queue.current_index, Some(0));
    }

    #[test]
    fn test_playback_state_default() {
        let state = PlaybackState::default();

        assert!(!state.is_playing);
        assert_eq!(state.current_time, 0.0);
        assert_eq!(state.playback_rate, 1.0);
        assert_eq!(state.volume, 1.0);
    }
}
