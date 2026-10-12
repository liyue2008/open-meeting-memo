use super::contracts::*;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    None,
    Start,
    Stop,
    Switch,
}
/// Pure signal timing; the service executes and reconciles actions with capture status.
pub struct Lifecycle {
    start_delay_ms: i64,
    end_grace_ms: i64,
    candidate: Option<(String, i64)>,
    active: Option<String>,
    absent_since: Option<i64>,
    suppressed: Option<String>,
}
impl Lifecycle {
    pub fn new(start_delay_ms: i64, end_grace_ms: i64) -> Self {
        Self {
            start_delay_ms,
            end_grace_ms,
            candidate: None,
            active: None,
            absent_since: None,
            suppressed: None,
        }
    }
    pub fn manual_stop(&mut self) {
        self.suppressed = self.active.take();
        self.absent_since = None;
    }
    pub fn reconcile_recording(&mut self, source_id: Option<String>) {
        if self.active != source_id {
            self.candidate = None;
        }
        self.active = source_id;
    }
    pub fn observe(&mut self, o: &MeetingObservation, consent: bool) -> Action {
        if o.state == MeetingState::Unknown {
            self.candidate = None;
            self.absent_since = None;
            return Action::None;
        }
        if o.state == MeetingState::Inactive && o.evidence == Evidence::Strong {
            self.candidate = None;
            if self.active.is_some() {
                let since = *self.absent_since.get_or_insert(o.observed_at_ms);
                if o.observed_at_ms.saturating_sub(since) >= self.end_grace_ms {
                    self.active = None;
                    self.absent_since = None;
                    return Action::Stop;
                }
            } else {
                self.suppressed = None;
            }
            return Action::None;
        }
        self.absent_since = None;
        if o.state != MeetingState::Active || o.evidence != Evidence::Strong || !consent {
            self.candidate = None;
            return Action::None;
        }
        let Some(id) = o.source_id.as_ref() else {
            self.candidate = None;
            return Action::None;
        };
        if self.active.as_ref() == Some(id) || self.suppressed.as_ref() == Some(id) {
            self.candidate = None;
            return Action::None;
        }
        let (candidate, since) = self
            .candidate
            .get_or_insert_with(|| (id.clone(), o.observed_at_ms));
        if candidate != id {
            *candidate = id.clone();
            *since = o.observed_at_ms;
        }
        if o.observed_at_ms.saturating_sub(*since) >= self.start_delay_ms {
            if self.active.is_some() {
                // Keep A until the service successfully seals it; a failed seal retries safely.
                return Action::Switch;
            }
            self.active = Some(id.clone());
            self.candidate = None;
            return Action::Start;
        }
        Action::None
    }
}
