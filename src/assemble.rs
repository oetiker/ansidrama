//! Turn the sampler's state log and the driver's input marks into frames.
//! Pure: no PTY, no clock, no rendering — every rule here is unit-tested.

use std::time::{Duration, Instant};

/// Where a frame's pixels come from.
#[derive(Debug)]
pub enum FrameSource {
    /// A committed state, by index into the state log.
    State(usize),
    /// A synthetic title card, by scene index.
    Card(usize),
    /// Reuse the previous frame's screen — pointer-only motion.
    Reuse,
}

#[derive(Debug)]
pub enum FrameKind {
    /// The settled result of an input: paced by the script.
    InputDriven,
    /// The app moving on its own: paced by the clock.
    AppDriven,
    Card,
}

#[derive(Debug)]
pub struct FrameSpec {
    pub source: FrameSource,
    pub kind: FrameKind,
    pub hold_cs: u16,
    pub mouse: Option<(u32, u32)>,
    pub scene: usize,
    /// The ordinal of the input within its scene (0 for the scene's first
    /// input, 1 for its second, ...). `Some` only on input-driven frames.
    pub input: Option<u32>,
}

/// One input the driver sent, and the state the wait settled on.
pub struct InputMark {
    pub t: Instant,
    pub scene: usize,
    pub settled: usize,
    pub authored_cs: u16,
    pub mouse: Option<(u32, u32)>,
    pub animated: bool,
}

pub enum Mark {
    Input(InputMark),
    Card { scene: usize, hold_cs: u16 },
    MouseMove { scene: usize, mouse: (u32, u32), hold_cs: u16 },
}

fn cs(d: Duration, min_cs: u16) -> u16 {
    let raw = (d.as_millis() / 10) as u16;
    raw.max(1).max(min_cs)
}

pub fn assemble(
    state_times: &[Instant],
    end: Instant,
    marks: &[Mark],
    min_cs: u16,
) -> Vec<FrameSpec> {
    // How long state i was on screen: until the next state, or until `end`.
    let measured = |i: usize| -> Duration {
        let next = state_times.get(i + 1).copied().unwrap_or(end);
        next.saturating_duration_since(state_times[i])
    };

    // The next input's timestamp bounds which states this input owns.
    let next_input_t: Vec<Instant> = {
        let ts: Vec<Instant> = marks
            .iter()
            .filter_map(|m| match m {
                Mark::Input(i) => Some(i.t),
                _ => None,
            })
            .collect();
        ts.iter()
            .enumerate()
            .map(|(k, _)| ts.get(k + 1).copied().unwrap_or(end))
            .collect()
    };

    let mut out = Vec::new();
    let mut input_ord = 0usize; // global index into next_input_t, across all scenes
    let mut scene_input_ord: std::collections::HashMap<usize, u32> = std::collections::HashMap::new();
    for m in marks {
        match m {
            Mark::Card { scene, hold_cs } => out.push(FrameSpec {
                source: FrameSource::Card(*scene),
                kind: FrameKind::Card,
                hold_cs: (*hold_cs).max(min_cs),
                mouse: None,
                scene: *scene,
                input: None,
            }),
            Mark::MouseMove { scene, mouse, hold_cs } => out.push(FrameSpec {
                source: FrameSource::Reuse,
                kind: FrameKind::AppDriven,
                hold_cs: (*hold_cs).max(min_cs),
                mouse: Some(*mouse),
                scene: *scene,
                input: None,
            }),
            Mark::Input(i) => {
                let upto = next_input_t[input_ord];
                input_ord += 1;

                // The per-scene ordinal: how many Mark::Input for this scene
                // came before this one. Distinct from `input_ord` above,
                // which counts across all scenes.
                let ord = scene_input_ord.entry(i.scene).or_insert(0);
                let this_ord = *ord;
                *ord += 1;

                let settled_in_window = state_times
                    .get(i.settled)
                    .is_some_and(|t| *t >= i.t && *t < upto);
                let any_state_in_window =
                    state_times.iter().any(|t| *t >= i.t && *t < upto);

                // Override 1: an input that changed nothing on screen has its
                // settled state predate it, so the state-scan below never
                // visits it. Every input still owes exactly one input-driven
                // frame, so emit it up front in that case.
                if !i.animated && !settled_in_window {
                    out.push(FrameSpec {
                        source: FrameSource::State(i.settled),
                        kind: FrameKind::InputDriven,
                        hold_cs: i.authored_cs.max(min_cs),
                        mouse: i.mouse,
                        scene: i.scene,
                        input: Some(this_ord),
                    });
                }

                // Override 3: an animated input runs no wait, so nothing is
                // ever pending and `settled` predates the window like above —
                // but here the state-scan below has nothing to visit at all
                // when the screen never moved during the dwell. Without this,
                // a screen that happens to hold still under `realtime` (or a
                // scene marked `animated`) contributes zero frames, and the
                // rule that every input owes at least one is broken. Emit one
                // app-driven frame, measured over the window's own duration —
                // the dwell becomes the measured hold, same as every other
                // frame in an animated scene.
                if i.animated && !any_state_in_window && state_times.get(i.settled).is_some() {
                    out.push(FrameSpec {
                        source: FrameSource::State(i.settled),
                        kind: FrameKind::AppDriven,
                        hold_cs: cs(upto.saturating_duration_since(i.t), min_cs),
                        mouse: i.mouse,
                        scene: i.scene,
                        input: None,
                    });
                }

                for (idx, t) in state_times.iter().enumerate() {
                    if *t < i.t || *t >= upto {
                        continue;
                    }
                    // Only true when settled_in_window, so this never
                    // double-emits the state pushed above.
                    let is_settled = idx == i.settled && !i.animated;
                    out.push(FrameSpec {
                        source: FrameSource::State(idx),
                        kind: if is_settled { FrameKind::InputDriven } else { FrameKind::AppDriven },
                        hold_cs: if is_settled {
                            i.authored_cs.max(min_cs)
                        } else {
                            cs(measured(idx), min_cs)
                        },
                        mouse: i.mouse,
                        scene: i.scene,
                        input: if is_settled { Some(this_ord) } else { None },
                    });
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    fn input(t: Instant, settled: usize, authored_cs: u16) -> Mark {
        Mark::Input(InputMark {
            t,
            scene: 0,
            settled,
            authored_cs,
            mouse: None,
            animated: false,
        })
    }

    #[test]
    fn the_settled_state_carries_the_authored_duration() {
        let t0 = Instant::now();
        let times = vec![t0 + ms(10)];
        let marks = vec![input(t0, 0, 28)];
        let f = assemble(&times, t0 + ms(500), &marks, 1);
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].hold_cs, 28, "authored, not measured");
        assert!(matches!(f[0].kind, FrameKind::InputDriven));
    }

    #[test]
    fn a_later_state_is_app_driven_at_measured_time() {
        let t0 = Instant::now();
        // settled at +10ms; the app moves again at +200ms and holds to +600ms
        let times = vec![t0 + ms(10), t0 + ms(200)];
        let marks = vec![input(t0, 0, 28)];
        let f = assemble(&times, t0 + ms(600), &marks, 1);
        assert_eq!(f.len(), 2);
        assert_eq!(f[0].hold_cs, 28);
        assert!(matches!(f[1].kind, FrameKind::AppDriven));
        assert_eq!(f[1].hold_cs, 40, "600ms - 200ms = 400ms = 40cs");
    }

    #[test]
    fn a_state_before_the_settled_one_is_also_app_driven() {
        let t0 = Instant::now();
        // a two-stage redraw: stage 1 at +10ms, the awaited stage 2 at +400ms
        let times = vec![t0 + ms(10), t0 + ms(400)];
        let marks = vec![input(t0, 1, 28)];
        let f = assemble(&times, t0 + ms(900), &marks, 1);
        assert_eq!(f.len(), 2);
        assert!(matches!(f[0].kind, FrameKind::AppDriven));
        assert_eq!(f[0].hold_cs, 39, "400ms - 10ms = 390ms = 39cs");
        assert!(matches!(f[1].kind, FrameKind::InputDriven));
        assert_eq!(f[1].hold_cs, 28);
    }

    #[test]
    fn states_are_scoped_to_the_input_that_owns_them() {
        let t0 = Instant::now();
        let times = vec![t0 + ms(10), t0 + ms(510)];
        let marks = vec![input(t0, 0, 9), input(t0 + ms(500), 1, 28)];
        let f = assemble(&times, t0 + ms(900), &marks, 1);
        assert_eq!(f.len(), 2, "one input-driven frame each, none stolen");
        assert_eq!(f[0].hold_cs, 9);
        assert_eq!(f[1].hold_cs, 28);
    }

    #[test]
    fn an_animated_scene_measures_every_frame() {
        let t0 = Instant::now();
        let times = vec![t0 + ms(10), t0 + ms(110), t0 + ms(210)];
        let marks = vec![Mark::Input(InputMark {
            t: t0,
            scene: 0,
            settled: 0,
            authored_cs: 99,
            mouse: None,
            animated: true,
        })];
        let f = assemble(&times, t0 + ms(310), &marks, 1);
        assert_eq!(f.len(), 3);
        assert!(f.iter().all(|s| matches!(s.kind, FrameKind::AppDriven)));
        assert!(f.iter().all(|s| s.hold_cs == 10), "all measured: {:?}", f.iter().map(|s| s.hold_cs).collect::<Vec<_>>());
    }

    #[test]
    fn mouse_move_frames_reuse_the_screen() {
        let t0 = Instant::now();
        let times = vec![t0 + ms(10)];
        let marks = vec![
            input(t0, 0, 28),
            Mark::MouseMove { scene: 0, mouse: (5, 5), hold_cs: 4 },
        ];
        let f = assemble(&times, t0 + ms(500), &marks, 1);
        assert_eq!(f.len(), 2);
        assert!(matches!(f[1].source, FrameSource::Reuse));
        assert_eq!(f[1].mouse, Some((5, 5)));
        assert_eq!(f[1].hold_cs, 4);
    }

    #[test]
    fn cards_are_emitted_in_order() {
        let t0 = Instant::now();
        let times = vec![t0 + ms(10)];
        let marks = vec![
            Mark::Card { scene: 0, hold_cs: 200 },
            input(t0, 0, 28),
        ];
        let f = assemble(&times, t0 + ms(500), &marks, 1);
        assert!(matches!(f[0].source, FrameSource::Card(0)));
        assert_eq!(f[0].hold_cs, 200);
    }

    #[test]
    fn min_cs_clamps_a_very_short_measured_frame() {
        let t0 = Instant::now();
        let times = vec![t0 + ms(10), t0 + ms(20)];
        let marks = vec![input(t0, 0, 28)];
        // 10ms measured = 1cs, but max_fps may demand 4cs
        let f = assemble(&times, t0 + ms(30), &marks, 4);
        assert_eq!(f[1].hold_cs, 4);
    }

    // --- Override 1: every input owes exactly one input-driven frame ---

    #[test]
    fn an_input_that_changed_nothing_still_gets_its_frame() {
        let t0 = Instant::now();
        // One state, at +10ms. Both inputs settle on it: the second input
        // changed nothing, so its settled state predates it.
        let times = vec![t0 + ms(10)];
        let marks = vec![input(t0, 0, 9), input(t0 + ms(500), 0, 28)];
        let f = assemble(&times, t0 + ms(900), &marks, 1);
        assert_eq!(f.len(), 2, "every input owes exactly one input-driven frame");
        assert!(f.iter().all(|s| matches!(s.kind, FrameKind::InputDriven)));
        assert_eq!(f[0].hold_cs, 9);
        assert_eq!(f[1].hold_cs, 28);
    }

    // --- Override 3: an animated input whose window holds no states still
    //     owes exactly one frame ---

    #[test]
    fn an_animated_input_with_no_states_in_its_window_still_gets_one_frame() {
        let t0 = Instant::now();
        // One state, well before the input; the input's own window (from its
        // own timestamp to the next input, here `end`) contains no states at
        // all — the screen never moved during the dwell.
        let times = vec![t0 + ms(1)];
        let marks = vec![Mark::Input(InputMark {
            t: t0 + ms(50),
            scene: 0,
            settled: 0,
            authored_cs: 99,
            mouse: None,
            animated: true,
        })];
        let f = assemble(&times, t0 + ms(150), &marks, 1);
        assert_eq!(f.len(), 1, "an animated input owes a frame even when nothing moved");
        assert!(matches!(f[0].kind, FrameKind::AppDriven));
        assert_eq!(f[0].hold_cs, 10, "150ms - 50ms = 100ms = 10cs");
        assert_eq!(f[0].input, None, "app-driven frames carry no input ordinal");
    }

    // --- Override 2: FrameSpec carries the per-scene input ordinal ---

    #[test]
    fn input_ordinal_restarts_per_scene_and_is_none_off_input_frames() {
        let t0 = Instant::now();
        let times = vec![t0 + ms(10), t0 + ms(510), t0 + ms(1010)];
        let marks = vec![
            // Scene 0: two inputs, ordinals 0 and 1.
            Mark::Input(InputMark {
                t: t0,
                scene: 0,
                settled: 0,
                authored_cs: 9,
                mouse: None,
                animated: false,
            }),
            Mark::Input(InputMark {
                t: t0 + ms(500),
                scene: 0,
                settled: 1,
                authored_cs: 28,
                mouse: None,
                animated: false,
            }),
            // Scene 1: a fresh input, ordinal restarts at 0.
            Mark::Input(InputMark {
                t: t0 + ms(1000),
                scene: 1,
                settled: 2,
                authored_cs: 15,
                mouse: None,
                animated: false,
            }),
            // An app-driven (mouse-move) frame carries no input ordinal.
            Mark::MouseMove { scene: 1, mouse: (1, 1), hold_cs: 3 },
        ];
        let f = assemble(&times, t0 + ms(1500), &marks, 1);
        assert_eq!(f.len(), 4);
        assert_eq!(f[0].input, Some(0), "scene 0's first input");
        assert_eq!(f[1].input, Some(1), "scene 0's second input");
        assert_eq!(f[2].input, Some(0), "scene 1's first input: ordinal restarts");
        assert_eq!(f[3].input, None, "app-driven mouse-move frame carries no ordinal");
    }
}
