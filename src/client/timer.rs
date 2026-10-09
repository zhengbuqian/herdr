use std::time::{Duration, Instant};

use super::ClientLoopEvent;

pub(super) struct ClientLoopTimer {
    deadline: Option<Instant>,
}

impl ClientLoopTimer {
    pub(super) fn new() -> Self {
        Self { deadline: None }
    }

    pub(super) fn deadline(&mut self, now: Instant, delay: Duration) -> Instant {
        let requested = now.checked_add(delay).unwrap_or(now);
        let deadline = self
            .deadline
            .map_or(requested, |current| current.min(requested));
        self.deadline = Some(deadline);
        deadline
    }

    pub(super) fn fired(&mut self) {
        self.deadline = None;
    }
}

/// Client-local inactivity timeout. Server frames and pane output never renew it.
pub(super) struct IdleDetach {
    timeout: Option<Duration>,
    last_user_activity: Instant,
}

impl IdleDetach {
    pub(super) fn new(minutes: u16, now: Instant) -> Self {
        let mut timer = Self {
            timeout: None,
            last_user_activity: now,
        };
        timer.configure(minutes, now);
        timer
    }

    pub(super) fn configure(&mut self, minutes: u16, now: Instant) {
        let timeout = (minutes != 0).then(|| Duration::from_secs(u64::from(minutes) * 60));
        if self.timeout != timeout {
            self.timeout = timeout;
            self.last_user_activity = now;
        }
    }

    pub(super) fn expired(&self, now: Instant) -> bool {
        self.timeout.is_some_and(|timeout| {
            now.saturating_duration_since(self.last_user_activity) >= timeout
        })
    }

    pub(super) fn observe_event(&mut self, event: &ClientLoopEvent, now: Instant) {
        if self.timeout.is_some() && client_event_has_user_activity(event) {
            self.last_user_activity = now;
        }
    }
}

fn client_event_has_user_activity(event: &ClientLoopEvent) -> bool {
    match event {
        #[cfg(unix)]
        ClientLoopEvent::StdinInput(data) => crate::raw_input::parse_raw_input_bytes_sync(data)
            .iter()
            .any(|event| {
                matches!(
                    event,
                    crate::raw_input::RawInputEvent::Key(_)
                        | crate::raw_input::RawInputEvent::Text(_)
                        | crate::raw_input::RawInputEvent::Paste(_)
                        | crate::raw_input::RawInputEvent::Mouse(_)
                        | crate::raw_input::RawInputEvent::OuterFocusGained
                )
            }),
        #[cfg(unix)]
        ClientLoopEvent::PixelMouse(_, _) => true,
        #[cfg(windows)]
        ClientLoopEvent::StdinEvents(events) => events.iter().any(|event| {
            matches!(
                event,
                crate::protocol::ClientInputEvent::Key { .. }
                    | crate::protocol::ClientInputEvent::TextCommit(_)
                    | crate::protocol::ClientInputEvent::Mouse { .. }
                    | crate::protocol::ClientInputEvent::Paste { .. }
                    | crate::protocol::ClientInputEvent::FocusGained
            )
        }),
        ClientLoopEvent::Resize(..) => true,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idle_detach_ignores_server_updates_and_timer_ticks() {
        let start = Instant::now();
        let mut idle = IdleDetach::new(1, start);
        idle.observe_event(&ClientLoopEvent::Timer, start + Duration::from_secs(30));
        idle.observe_event(
            &ClientLoopEvent::ServerMessage {
                endpoint_id: crate::client::endpoint::ClientEndpointId::Local,
                generation: 1,
                message: Box::new(crate::protocol::ServerMessage::ReloadSoundConfig),
            },
            start + Duration::from_secs(59),
        );
        assert!(!idle.expired(start + Duration::from_secs(59)));
        assert!(idle.expired(start + Duration::from_secs(60)));
    }

    #[cfg(unix)]
    #[test]
    fn idle_detach_renews_on_user_input_but_not_terminal_replies() {
        let start = Instant::now();
        for input in [b"x".as_slice(), b"\x1b[I", b"\x1b[200~paste\x1b[201~"] {
            let mut idle = IdleDetach::new(1, start);
            idle.observe_event(
                &ClientLoopEvent::StdinInput(input.to_vec()),
                start + Duration::from_secs(50),
            );
            idle.observe_event(
                &ClientLoopEvent::StdinInput(b"\x1b[8;40;120t".to_vec()),
                start + Duration::from_secs(100),
            );
            assert!(!idle.expired(start + Duration::from_secs(109)), "{input:?}");
            assert!(idle.expired(start + Duration::from_secs(110)), "{input:?}");
        }
    }

    #[test]
    fn idle_detach_reload_preserves_elapsed_time_unless_timeout_changes() {
        let start = Instant::now();
        let mut idle = IdleDetach::new(1, start);
        idle.configure(1, start + Duration::from_secs(59));
        assert!(idle.expired(start + Duration::from_secs(60)));
        idle.configure(2, start + Duration::from_secs(60));
        assert!(!idle.expired(start + Duration::from_secs(179)));
        assert!(idle.expired(start + Duration::from_secs(180)));
        idle.configure(0, start + Duration::from_secs(180));
        assert!(!idle.expired(start + Duration::from_secs(100_000)));
    }

    #[test]
    fn incoming_events_do_not_postpone_timer_deadline() {
        let start = Instant::now();
        let delay = Duration::from_millis(100);
        let mut timer = ClientLoopTimer::new();

        let first_deadline = timer.deadline(start, delay);
        assert_eq!(
            timer.deadline(start + Duration::from_millis(25), delay),
            first_deadline
        );
        assert_eq!(
            timer.deadline(start + Duration::from_millis(50), delay),
            first_deadline
        );
        assert_eq!(
            timer.deadline(start + Duration::from_millis(75), delay),
            first_deadline
        );

        timer.fired();
        assert_eq!(
            timer.deadline(first_deadline, delay),
            first_deadline + delay
        );
    }

    #[test]
    fn earlier_client_work_can_pull_the_timer_deadline_forward() {
        let start = Instant::now();
        let mut timer = ClientLoopTimer::new();

        assert_eq!(
            timer.deadline(start, Duration::from_millis(100)),
            start + Duration::from_millis(100)
        );
        assert_eq!(
            timer.deadline(start + Duration::from_millis(20), Duration::from_millis(10)),
            start + Duration::from_millis(30)
        );
        assert_eq!(
            timer.deadline(
                start + Duration::from_millis(25),
                Duration::from_millis(100)
            ),
            start + Duration::from_millis(30)
        );
    }
}
