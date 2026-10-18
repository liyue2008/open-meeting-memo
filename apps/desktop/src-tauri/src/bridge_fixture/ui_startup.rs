//! Admission for the exact complete-UI fixture; never a business-success assertion.
use serde_json::{json, Value};
use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

const DRIVER: &str = include_str!("../../../../../scripts/fixtures/desktop-ui-probe.js");
const LIMIT: Duration = Duration::from_secs(20);
pub const BOOTSTRAP: &str = r#"(()=>{
  let frames=0, pending=null, active=true, error=false, cleanup=0;
  const facts=()=>{
    let visible=false,focused=false,viewport=false;
    try {
      visible=document.visibilityState==='visible'; focused=document.hasFocus()===true;
      viewport=Number.isFinite(window.innerWidth)&&Number.isFinite(window.innerHeight)&&window.innerWidth>0&&window.innerHeight>0;
    } catch (_) { error=true; }
    return {frames,visible,focused,viewport,error,cleanup};
  };
  const draw=()=>{
    pending=null;
    if(!active)return;
    frames=Math.min(2,frames+1);
    if(frames<2) schedule(); else active=false;
  };
  const schedule=()=>{
    try { pending=window.requestAnimationFrame(draw); }
    catch (_) { pending=null; error=true; active=false; }
  };
  const cancel=()=>{
    active=false;
    if(pending!==null){
      try{window.cancelAnimationFrame(pending);pending=null;cleanup=1;}
      catch(_){cleanup=2;}
    }
    return facts();
  };
  window.__ommFixtureUiStartup={read:facts,cancel};
  if(typeof window.requestAnimationFrame!=='function'||typeof window.cancelAnimationFrame!=='function'){
    error=true;active=false;
  }else{schedule();}
  return JSON.stringify(facts());
})()"#;
const READ: &str = "JSON.stringify(window.__ommFixtureUiStartup.read())";
// An absent bootstrap is not proof that an earlier queued evaluation cannot still arrive.
const CANCEL: &str = "JSON.stringify(window.__ommFixtureUiStartup?window.__ommFixtureUiStartup.cancel():{cleanup:2})";

pub fn selected(scenario: &str, driver: &str, store: bool, review: bool, archive: bool) -> bool {
    scenario == "ui" && driver == DRIVER && !store && !review && !archive
}

#[derive(Clone, Copy, Default)]
struct Probe {
    frames: u64,
    visible: bool,
    focused: bool,
    viewport: bool,
    error: bool,
}
impl Probe {
    fn parse(text: &str) -> Option<Self> {
        let Value::String(text) = serde_json::from_str(text).ok()? else {
            return None;
        };
        let v: Value = serde_json::from_str(&text).ok()?;
        let frames = v["frames"].as_u64().filter(|n| *n <= 2)?;
        Some(Self {
            frames,
            visible: v["visible"].as_bool()?,
            focused: v["focused"].as_bool()?,
            viewport: v["viewport"].as_bool()?,
            error: v["error"].as_bool()?,
        })
    }
    fn ready(self) -> bool {
        self.frames == 2 && self.visible && self.focused && self.viewport && !self.error
    }
}
#[derive(Debug, PartialEq, Eq)]
pub enum Action {
    Wait,
    Window,
    Probe,
    Start,
    Running,
    Cancel,
    Fail,
}
#[derive(Default)]
struct State {
    setup_at: Option<Instant>,
    setup_done: bool,
    finished: bool,
    worker: bool,
    window_requested: bool,
    window_claimed: bool,
    pending: bool,
    native: Option<(bool, bool)>,
    js: Option<Probe>,
    running: bool,
    failure: Option<&'static str>,
    cancel_requested: bool,
    reported: bool,
    cleanup: Option<u64>,
}
#[derive(Default)]
pub struct Controller {
    state: Mutex<State>,
}
impl Controller {
    pub fn setup_started(&self, now: Instant) {
        let mut s = self.state.lock().unwrap();
        if s.setup_at.is_none() {
            s.setup_at = Some(now);
        }
    }
    pub fn setup_complete(&self) {
        self.state.lock().unwrap().setup_done = true;
    }
    pub fn finished(&self) {
        self.state.lock().unwrap().finished = true;
    }
    pub fn claim_worker(&self) -> bool {
        let mut s = self.state.lock().unwrap();
        if s.worker {
            false
        } else {
            s.worker = true;
            true
        }
    }
    fn active(s: &State, now: Instant) -> bool {
        !s.running
            && s.failure.is_none()
            && s.setup_at
                .is_some_and(|at| now.saturating_duration_since(at) < LIMIT)
    }
    fn accepting(&self) -> bool {
        Self::active(&self.state.lock().unwrap(), Instant::now())
    }
    fn claim_window(&self, now: Instant) -> bool {
        let mut s = self.state.lock().unwrap();
        if !Self::active(&s, now) || !s.window_requested || s.window_claimed {
            return false;
        }
        s.window_claimed = true;
        true
    }
    pub fn missing_window(&self) -> bool {
        let mut s = self.state.lock().unwrap();
        if s.running || s.reported {
            return false;
        }
        if s.failure.is_none() {
            s.failure = Some("main-window-missing");
        }
        if s.worker {
            return false;
        }
        // No worker could have installed the bootstrap; this caller owns the sole failure.
        s.worker = true;
        s.reported = true;
        s.cleanup = Some(0);
        true
    }
    fn fail(&self, reason: &'static str) {
        let mut s = self.state.lock().unwrap();
        if !s.running && s.failure.is_none() {
            s.failure = Some(reason);
        }
    }
    fn native(&self, result: Result<(bool, bool), ()>, now: Instant) {
        let mut s = self.state.lock().unwrap();
        if !Self::active(&s, now) {
            return;
        }
        match result {
            Ok(v) => s.native = Some(v),
            Err(()) => s.failure = Some("window-operation"),
        }
    }
    fn receive(&self, text: &str, now: Instant) {
        let mut s = self.state.lock().unwrap();
        if !Self::active(&s, now) {
            return;
        }
        match Probe::parse(text) {
            Some(p) => {
                s.js = Some(p);
                if p.error {
                    s.failure = Some("document-read");
                }
            }
            None => s.failure = Some("document-read"),
        }
    }
    pub fn next(&self, now: Instant) -> Action {
        let mut s = self.state.lock().unwrap();
        if s.running {
            return Action::Running;
        }
        if s.reported {
            return Action::Wait;
        }
        if s.setup_at
            .is_some_and(|at| now.saturating_duration_since(at) >= LIMIT)
            && s.failure.is_none()
        {
            s.failure = Some("deadline");
        }
        if s.failure.is_some() {
            if !s.cancel_requested {
                s.cancel_requested = true;
                return Action::Cancel;
            }
            s.reported = true;
            return Action::Fail;
        }
        if !s.setup_done || !s.finished {
            return Action::Wait;
        }
        if !s.window_requested {
            s.window_requested = true;
            s.pending = true;
            return Action::Window;
        }
        if s.pending {
            let (Some(native), Some(js)) = (s.native, s.js) else {
                return Action::Wait;
            };
            if native == (true, true) && js.ready() {
                s.running = true;
                return Action::Start;
            }
            s.pending = false;
        }
        s.native = None;
        s.js = None;
        s.pending = true;
        Action::Probe
    }
    fn cleanup(&self, text: &str) {
        let parsed = serde_json::from_str::<Value>(text).ok().and_then(|v| {
            v.as_str()
                .and_then(|s| serde_json::from_str::<Value>(s).ok())
        });
        self.state.lock().unwrap().cleanup =
            parsed.and_then(|v| v["cleanup"].as_u64().filter(|n| *n <= 2));
    }
    pub fn evidence(&self) -> Value {
        let s = self.state.lock().unwrap();
        let p = s.js.unwrap_or_default();
        json!({"schemaVersion":1,"reason":if s.running {"admitted"}else{s.failure.unwrap_or("poll-budget")},"windowRequested":s.window_requested,"nativeRead":if s.native.is_some(){"available"}else{"unavailable"},"nativeVisible":s.native.is_some_and(|v|v.0),"nativeFocused":s.native.is_some_and(|v|v.1),"documentRead":if s.js.is_some(){"available"}else{"unavailable"},"documentVisible":p.visible,"documentFocused":p.focused,"viewportPositiveFinite":p.viewport,"frames":p.frames,"cleanup":if s.running {"not-needed"}else{match s.cleanup {Some(0)=>"not-needed",Some(1)=>"cancelled",_=>"unknown"}}})
    }
    pub fn failure(&self) -> Value {
        json!({"ok":false,"checks":[],"playCalls":0,"error":"ui-startup-not-ready","uiStartup":self.evidence()})
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WindowOp {
    Show,
    Focus,
    Visible,
    Focused,
}
fn window_check(
    show: bool,
    mut operation: impl FnMut(WindowOp) -> Result<bool, ()>,
) -> Result<(bool, bool), ()> {
    if show {
        operation(WindowOp::Show)?;
        operation(WindowOp::Focus)?;
    }
    Ok((operation(WindowOp::Visible)?, operation(WindowOp::Focused)?))
}
// Queue from the existing completion worker; no synchronous window calls in WK callbacks.
pub fn observe(controller: &Arc<Controller>, view: &tauri::Webview<tauri::Wry>, show: bool) {
    let gate = controller.clone();
    let app = view.app_handle().clone();
    let owned = view.clone();
    use tauri::Manager;
    let queued = view.app_handle().run_on_main_thread(move || {
        if !(if show {
            gate.claim_window(Instant::now())
        } else {
            gate.accepting()
        }) {
            return;
        }
        let result = (|| {
            let window = app.get_webview_window("main").ok_or(())?;
            if !crate::host::trusted(&window) {
                return Err(());
            }
            window_check(show, |op| match op {
                WindowOp::Show => window.show().map(|()| true).map_err(|_| ()),
                WindowOp::Focus => window.set_focus().map(|()| true).map_err(|_| ()),
                WindowOp::Visible => window.is_visible().map_err(|_| ()),
                WindowOp::Focused => window.is_focused().map_err(|_| ()),
            })
        })();
        gate.native(result, Instant::now());
        if !gate.accepting() {
            return;
        }
        let reply = gate.clone();
        if owned
            .eval_with_callback(if show { BOOTSTRAP } else { READ }, move |text| {
                reply.receive(&text, Instant::now())
            })
            .is_err()
        {
            gate.fail("document-dispatch");
        }
    });
    if queued.is_err() {
        controller.fail("window-dispatch");
    }
}
pub fn cancel(controller: &Arc<Controller>, view: &tauri::Webview<tauri::Wry>) {
    let gate = controller.clone();
    // One original worker iteration later reports unknown if this callback never arrives.
    let _ = view.eval_with_callback(CANCEL, move |text| gate.cleanup(&text));
}

#[cfg(test)]
mod tests {
    use super::*;
    fn reply(frames: u64, visible: bool, focused: bool, viewport: bool) -> String {
        serde_json::to_string(&json!({"frames":frames,"visible":visible,"focused":focused,"viewport":viewport,"error":false}).to_string()).unwrap()
    }
    #[test]
    fn exact_complete_ui_only() {
        assert!(selected("ui", DRIVER, false, false, false));
        for (scenario, driver, store, review, archive) in [
            ("native", DRIVER, false, false, false),
            ("window-acl", DRIVER, false, false, false),
            ("ui", "preferences", false, false, false),
            ("ui", "lifecycle", false, false, false),
            ("ui", "recovery", false, false, false),
            ("store", DRIVER, false, false, false),
            ("ui", DRIVER, true, false, false),
            ("ui", DRIVER, false, true, false),
            ("ui", DRIVER, false, false, true),
        ] {
            assert!(!selected(scenario, driver, store, review, archive));
        }
        assert!(!selected("ui", &format!("{DRIVER}\n"), false, false, false));
    }
    #[test]
    fn rendezvous_once_and_native_and_document_admission() {
        for finished_first in [false, true] {
            let g = Controller::default();
            let now = Instant::now();
            if finished_first {
                g.finished();
            }
            assert_eq!(g.next(now), Action::Wait);
            g.setup_started(now);
            if !finished_first {
                g.setup_complete();
                assert_eq!(g.next(now), Action::Wait);
                g.finished();
            } else {
                g.setup_complete();
            }
            assert!(g.claim_worker());
            assert!(!g.claim_worker());
            assert_eq!(g.next(now), Action::Window);
            assert_eq!(g.next(now), Action::Wait);
            for (native, frames, visible, focused, viewport) in [
                ((false, true), 2, true, true, true),
                ((true, false), 2, true, true, true),
                ((true, true), 0, true, true, true),
                ((true, true), 1, true, true, true),
                ((true, true), 2, false, true, true),
                ((true, true), 2, true, false, true),
                ((true, true), 2, true, true, false),
            ] {
                g.native(Ok(native), now);
                g.receive(&reply(frames, visible, focused, viewport), now);
                assert_eq!(g.next(now), Action::Probe);
            }
            g.native(Ok((true, true)), now);
            g.receive(&reply(2, true, true, true), now);
            assert_eq!(g.next(now), Action::Start);
            g.finished();
            g.setup_complete();
            g.setup_started(now + LIMIT);
            assert_eq!(g.next(now + LIMIT), Action::Running);
        }
    }
    #[test]
    fn deadline_no_finished_no_reply_and_late_events_never_start() {
        for finished in [false, true] {
            let g = Controller::default();
            let now = Instant::now();
            g.setup_started(now);
            g.setup_complete();
            if finished {
                g.finished();
                assert_eq!(g.next(now), Action::Window);
            }
            assert_eq!(g.next(now + LIMIT - Duration::from_nanos(1)), Action::Wait);
            g.setup_started(now + Duration::from_secs(10));
            assert_eq!(g.next(now + LIMIT), Action::Cancel);
            g.native(Ok((true, true)), now + LIMIT);
            g.receive(&reply(2, true, true, true), now + LIMIT);
            g.finished();
            g.setup_complete();
            assert_eq!(g.next(now + LIMIT), Action::Fail);
            assert_eq!(g.failure()["uiStartup"]["cleanup"], "unknown");
            assert_eq!(g.failure()["error"], "ui-startup-not-ready");
            assert_eq!(g.failure()["checks"], json!([]));
            assert_eq!(g.next(now + LIMIT), Action::Wait);
        }
    }
    #[test]
    fn actual_window_operation_sequence_with_fake_window_and_late_claim() {
        let mut calls = Vec::new();
        assert_eq!(
            window_check(true, |op| {
                calls.push(op);
                Ok(true)
            }),
            Ok((true, true))
        );
        assert_eq!(
            calls,
            vec![
                WindowOp::Show,
                WindowOp::Focus,
                WindowOp::Visible,
                WindowOp::Focused
            ]
        );
        calls.clear();
        assert_eq!(
            window_check(false, |op| {
                calls.push(op);
                Ok(false)
            }),
            Ok((false, false))
        );
        assert_eq!(calls, vec![WindowOp::Visible, WindowOp::Focused]);
        for rejected in [WindowOp::Show, WindowOp::Focus] {
            calls.clear();
            assert_eq!(
                window_check(true, |op| {
                    calls.push(op);
                    if op == rejected {
                        Err(())
                    } else {
                        Ok(true)
                    }
                }),
                Err(())
            );
            assert_eq!(calls.last(), Some(&rejected));
        }
        let g = Controller::default();
        let now = Instant::now();
        g.setup_started(now);
        g.setup_complete();
        g.finished();
        assert_eq!(g.next(now), Action::Window);
        assert!(g.claim_window(now));
        assert!(!g.claim_window(now));
        let late = Controller::default();
        late.setup_started(now);
        late.setup_complete();
        late.finished();
        assert_eq!(late.next(now), Action::Window);
        assert!(!late.claim_window(now + LIMIT));
        let missing = Controller::default();
        assert!(missing.missing_window());
        assert!(!missing.missing_window());
        assert!(!missing.claim_worker());
        assert_eq!(missing.failure()["uiStartup"]["cleanup"], "not-needed");
        let existing = Controller::default();
        assert!(existing.claim_worker());
        assert!(!existing.missing_window());
        assert_eq!(existing.failure()["uiStartup"]["cleanup"], "unknown");
    }
    #[test]
    fn fixed_errors_cleanup_and_untrusted_values() {
        for response in [
            "not-json".to_string(),
            reply(3, true, true, true),
            serde_json::to_string(
                &json!({"frames":2,"visible":true,"focused":true,"viewport":true,"error":true})
                    .to_string(),
            )
            .unwrap(),
        ] {
            let g = Controller::default();
            let now = Instant::now();
            g.setup_started(now);
            g.setup_complete();
            g.finished();
            assert_eq!(g.next(now), Action::Window);
            g.receive(&response, now);
            assert_eq!(g.next(now), Action::Cancel);
            g.cleanup(&serde_json::to_string("{\"cleanup\":1}").unwrap());
            assert_eq!(g.next(now), Action::Fail);
            assert_eq!(g.failure()["uiStartup"]["cleanup"], "cancelled");
            assert!(!g.failure().to_string().contains("not-json"));
        }
        let g = Controller::default();
        let now = Instant::now();
        g.setup_started(now);
        g.native(Err(()), now);
        assert_eq!(g.next(now), Action::Cancel);
        assert_eq!(g.next(now), Action::Fail);
        assert_eq!(g.failure()["uiStartup"]["reason"], "window-operation");
    }
}
