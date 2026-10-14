//! Keeps the filter on the microphone Windows uses. Plugging another
//! microphone in or out changes the default one, and a microphone on another
//! USB port gets a new endpoint id: kyrr has to move with it, or the filter
//! quietly stays on a microphone nobody speaks into.
//!
//! This only decides; the app polls it every couple of seconds and acts on
//! the answer. Kept free of Windows types so it can be tested anywhere.

/// Ticks in a row a change must hold before kyrr acts: Windows needs a moment
/// to make a newly plugged microphone the default, and the list flickers
/// while the audio service restarts.
const STABLE_TICKS: u32 = 5;

/// What the app sees right now.
pub struct Input<'a> {
    /// Endpoint GUID the filter is on.
    pub attached: Option<&'a str>,
    /// The filter's microphone is plugged in.
    pub attached_present: bool,
    /// The effect slot no longer holds kyrr (a Windows or driver update).
    pub detached: bool,
    /// Endpoint GUID of the microphone Windows uses by default.
    pub default: Option<&'a str>,
    /// The app must not touch the setup now (first-run, setting up, no filter).
    pub busy: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    Idle,
    /// Move the filter to this microphone now.
    Move(String),
    /// Moving by itself did not work out (or was declined): show a button.
    Offer {
        to: String,
        unplugged: bool,
    },
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Need {
    /// The default microphone is another one.
    Switch,
    /// Right microphone, but Windows took kyrr off it.
    Repair,
}

#[derive(Default)]
pub struct Follower {
    /// The default microphone at the last tick that knew one.
    last_default: Option<String>,
    /// How many ticks in a row the same need has stood.
    stable: u32,
    /// Microphones kyrr already tried to move to by itself, until the filter
    /// is on them: no second automatic prompt for them, however often the
    /// default flips in between.
    tried: Vec<String>,
    /// The default microphone while the user chose another one on purpose:
    /// left alone while the default is that one. Only a move away from the
    /// pick ends it, so a default that flickers does not cost the pick.
    pinned: Option<String>,
}

fn same(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

impl Follower {
    /// Starts with the pick of an earlier run (see [`Follower::pinned`]).
    pub fn with_pinned(pinned: Option<String>) -> Follower {
        Follower {
            pinned,
            ..Follower::default()
        }
    }

    /// The default microphone the user's own pick holds against, to keep
    /// across restarts.
    pub fn pinned(&self) -> Option<&str> {
        self.pinned.as_deref()
    }

    /// The user picked a microphone by hand while `default` is the default:
    /// keep it until Windows' default changes.
    pub fn user_chose(&mut self, default: Option<&str>) {
        self.pinned = default.map(str::to_string);
        self.stable = 0;
    }

    pub fn tick(&mut self, input: &Input) -> Outcome {
        // Setup is running, or Windows has no microphone to name right now (the
        // audio service restarts, a headset reconnects): nothing is learned
        // and nothing is decided, so the state of before survives it.
        let Some(default) = input.default.filter(|_| !input.busy) else {
            self.stable = 0;
            return Outcome::Idle;
        };
        let first_sight = self.last_default.is_none();
        if self.last_default.as_deref().is_some_and(|l| !same(l, default)) {
            self.stable = 0;
        }
        self.last_default = Some(default.to_string());
        let need = match input.attached {
            Some(a) if !same(a, default) => Some(Need::Switch),
            Some(_) if input.detached && input.attached_present => Some(Need::Repair),
            _ => None,
        };
        let Some(need) = need else {
            self.stable = 0;
            // Done with a microphone the filter is on now; one that was
            // declined or failed stays remembered, so a default flipping to
            // and fro does not ask for it again.
            if let Some(a) = input.attached {
                self.tried.retain(|t| !same(t, a));
            }
            return Outcome::Idle;
        };
        // A microphone chosen on purpose stays, as long as it is there. A
        // run that finds one already apart from the default (kyrr was set up
        // so before this existed) takes it for a choice too.
        if need == Need::Switch && input.attached_present {
            if first_sight && self.pinned.is_none() {
                self.pinned = Some(default.to_string());
            }
            if self.pinned.as_deref().is_some_and(|p| same(p, default)) {
                self.stable = 0;
                return Outcome::Idle;
            }
        }
        self.stable = self.stable.saturating_add(1);
        if self.stable < STABLE_TICKS {
            return Outcome::Idle;
        }
        let to = default.to_string();
        if !self.tried.iter().any(|t| same(t, &to)) {
            self.tried.push(to.clone());
            // Leaving the microphone the user picked: that pick is over.
            self.pinned = None;
            return Outcome::Move(to);
        }
        match need {
            Need::Switch => Outcome::Offer {
                to,
                unplugged: !input.attached_present,
            },
            // The Repair button on the main page covers this one.
            Need::Repair => Outcome::Idle,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIFINE: &str = "{aaaaaaaa-0000-0000-0000-000000000001}";
    const PODMIC: &str = "{bbbbbbbb-0000-0000-0000-000000000002}";

    fn input<'a>(attached: Option<&'a str>, present: bool, default: Option<&'a str>) -> Input<'a> {
        Input {
            attached,
            attached_present: present,
            detached: false,
            default,
            busy: false,
        }
    }

    /// Ticks until something other than Idle comes back, at most `n` times.
    fn run(f: &mut Follower, i: &Input, n: u32) -> Outcome {
        for _ in 0..n {
            let o = f.tick(i);
            if o != Outcome::Idle {
                return o;
            }
        }
        Outcome::Idle
    }

    #[test]
    fn on_the_default_microphone_nothing_happens() {
        let mut f = Follower::default();
        assert_eq!(run(&mut f, &input(Some(FIFINE), true, Some(FIFINE)), 20), Outcome::Idle);
    }

    #[test]
    fn follows_a_new_default_after_a_few_ticks() {
        let mut f = Follower::default();
        let i = input(Some(FIFINE), false, Some(PODMIC));
        for _ in 1..STABLE_TICKS {
            assert_eq!(f.tick(&i), Outcome::Idle);
        }
        assert_eq!(f.tick(&i), Outcome::Move(PODMIC.into()));
    }

    #[test]
    fn a_flicker_is_not_followed() {
        let mut f = Follower::default();
        let away = input(Some(FIFINE), true, Some(PODMIC));
        let back = input(Some(FIFINE), true, Some(FIFINE));
        for _ in 1..STABLE_TICKS {
            assert_eq!(f.tick(&away), Outcome::Idle);
        }
        assert_eq!(f.tick(&back), Outcome::Idle);
        for _ in 1..STABLE_TICKS {
            assert_eq!(f.tick(&away), Outcome::Idle);
        }
    }

    #[test]
    fn moves_once_then_offers_a_button() {
        let mut f = Follower::default();
        let i = input(Some(FIFINE), false, Some(PODMIC));
        assert_eq!(run(&mut f, &i, STABLE_TICKS), Outcome::Move(PODMIC.into()));
        // Still not moved (declined, failed): a button, no second prompt.
        assert_eq!(
            f.tick(&i),
            Outcome::Offer {
                to: PODMIC.into(),
                unplugged: true
            }
        );
        assert_eq!(
            f.tick(&i),
            Outcome::Offer {
                to: PODMIC.into(),
                unplugged: true
            }
        );
    }

    #[test]
    fn plugging_back_and_forth_follows_every_time() {
        let mut f = Follower::default();
        f.tick(&input(Some(FIFINE), true, Some(FIFINE)));
        let to_pod = input(Some(FIFINE), true, Some(PODMIC));
        assert_eq!(run(&mut f, &to_pod, STABLE_TICKS), Outcome::Move(PODMIC.into()));
        // Done: kyrr is on the PodMic now.
        let on_pod = input(Some(PODMIC), true, Some(PODMIC));
        assert_eq!(run(&mut f, &on_pod, 5), Outcome::Idle);
        // And back to the fifine.
        let to_fifine = input(Some(PODMIC), true, Some(FIFINE));
        assert_eq!(run(&mut f, &to_fifine, STABLE_TICKS), Outcome::Move(FIFINE.into()));
    }

    #[test]
    fn a_microphone_chosen_by_hand_stays() {
        let mut f = Follower::default();
        // The default is the PodMic, the user chose the fifine on purpose.
        let i = input(Some(FIFINE), true, Some(PODMIC));
        f.user_chose(Some(PODMIC));
        assert_eq!(run(&mut f, &i, 20), Outcome::Idle);
        // Unplugged: that choice is over.
        let gone = input(Some(FIFINE), false, Some(PODMIC));
        assert_eq!(run(&mut f, &gone, STABLE_TICKS), Outcome::Move(PODMIC.into()));
        // A new default is followed again.
        let mut f = Follower::default();
        f.tick(&input(Some(FIFINE), true, Some(FIFINE)));
        f.user_chose(Some(PODMIC));
        let other = input(Some(FIFINE), true, Some("{cccccccc-0000-0000-0000-000000000003}"));
        assert!(matches!(run(&mut f, &other, STABLE_TICKS), Outcome::Move(_)));
    }

    #[test]
    fn repairs_once_by_itself() {
        let mut f = Follower::default();
        let mut i = input(Some(FIFINE), true, Some(FIFINE));
        i.detached = true;
        assert_eq!(run(&mut f, &i, STABLE_TICKS), Outcome::Move(FIFINE.into()));
        assert_eq!(run(&mut f, &i, 10), Outcome::Idle);
    }

    #[test]
    fn leaves_setup_alone() {
        let mut f = Follower::default();
        let mut i = input(Some(FIFINE), false, Some(PODMIC));
        i.busy = true;
        assert_eq!(run(&mut f, &i, 20), Outcome::Idle);
        // Without a filter anywhere (never set up) there is nothing to move.
        assert_eq!(run(&mut f, &input(None, false, Some(PODMIC)), 20), Outcome::Idle);
        // No default microphone at all: nothing to move to.
        assert_eq!(run(&mut f, &input(Some(FIFINE), false, None), 20), Outcome::Idle);
    }

    #[test]
    fn guids_compare_regardless_of_case() {
        let mut f = Follower::default();
        let upper = FIFINE.to_uppercase();
        assert_eq!(run(&mut f, &input(Some(FIFINE), true, Some(&upper)), 20), Outcome::Idle);
    }

    #[test]
    fn a_missing_default_changes_nothing() {
        // The audio service restarts: for a moment Windows names no microphone.
        let mut f = Follower::default();
        f.tick(&input(Some(FIFINE), true, Some(PODMIC)));
        f.user_chose(Some(PODMIC));
        assert_eq!(f.tick(&input(Some(FIFINE), true, None)), Outcome::Idle);
        assert_eq!(f.pinned(), Some(PODMIC));
        assert_eq!(run(&mut f, &input(Some(FIFINE), true, Some(PODMIC)), 30), Outcome::Idle);
    }

    #[test]
    fn a_failing_move_is_not_asked_again_and_again() {
        let mut f = Follower::default();
        let i = input(Some(FIFINE), false, Some(PODMIC));
        assert_eq!(run(&mut f, &i, STABLE_TICKS), Outcome::Move(PODMIC.into()));
        // Audio restarts around the attempt, the list is empty once.
        f.tick(&input(Some(FIFINE), false, None));
        for _ in 0..30 {
            assert!(matches!(f.tick(&i), Outcome::Idle | Outcome::Offer { .. }));
        }
    }

    #[test]
    fn a_flapping_default_asks_once_per_microphone() {
        let mut f = Follower::default();
        let built_in = "{dddddddd-0000-0000-0000-000000000004}";
        f.tick(&input(Some(PODMIC), true, Some(PODMIC)));
        let to_built_in = input(Some(PODMIC), true, Some(built_in));
        assert_eq!(run(&mut f, &to_built_in, STABLE_TICKS), Outcome::Move(built_in.into()));
        // It did not work out and Windows flips back and forth.
        let to_pod = input(Some(PODMIC), true, Some(PODMIC));
        f.tick(&to_pod);
        assert!(matches!(
            run(&mut f, &to_built_in, 30),
            Outcome::Idle | Outcome::Offer { .. }
        ));
    }

    #[test]
    fn a_microphone_apart_from_the_default_at_start_is_taken_for_a_choice() {
        // kyrr was set up on the fifine, Windows uses the PodMic: that is how
        // it was left, so it stays.
        let mut f = Follower::default();
        assert_eq!(run(&mut f, &input(Some(FIFINE), true, Some(PODMIC)), 30), Outcome::Idle);
        assert_eq!(f.pinned(), Some(PODMIC));
        // A pick kept from the last run holds as well.
        let mut f = Follower::with_pinned(Some(PODMIC.into()));
        assert_eq!(run(&mut f, &input(Some(FIFINE), true, Some(PODMIC)), 30), Outcome::Idle);
    }

    #[test]
    fn a_flickering_default_does_not_cost_the_pick() {
        let mut f = Follower::default();
        f.user_chose(Some(PODMIC));
        let flicker = input(Some(FIFINE), true, Some("{cccccccc-0000-0000-0000-000000000003}"));
        for _ in 0..STABLE_TICKS - 1 {
            assert_eq!(f.tick(&flicker), Outcome::Idle);
        }
        assert_eq!(f.tick(&input(Some(FIFINE), true, Some(PODMIC))), Outcome::Idle);
        assert_eq!(f.pinned(), Some(PODMIC));
        assert_eq!(run(&mut f, &input(Some(FIFINE), true, Some(PODMIC)), 30), Outcome::Idle);
    }

    #[test]
    fn an_old_pick_does_not_hold_the_filter_on_a_microphone_it_left() {
        // The pick (default was D) was kept from an earlier run, the default is X now.
        let mut f = Follower::with_pinned(Some(PODMIC.into()));
        let x = "{cccccccc-0000-0000-0000-000000000003}";
        assert_eq!(
            run(&mut f, &input(Some(FIFINE), true, Some(x)), STABLE_TICKS),
            Outcome::Move(x.into())
        );
        // Later the default is the PodMic again: the filter follows it.
        f.tick(&input(Some(x), true, Some(x)));
        assert_eq!(
            run(&mut f, &input(Some(x), true, Some(PODMIC)), STABLE_TICKS),
            Outcome::Move(PODMIC.into())
        );
    }
}
