//! Moving the filter from one microphone to another without ever ending up
//! with none: the new one is checked and attached first, the old ones are let
//! go only after that. A microphone pulled out while Windows asks for
//! permission (or while the change runs) then costs nothing: the filter stays
//! where it was, and the next look at the default microphone tries again.

/// Moves the filter to `target`. `attached` are the microphones it is on now;
/// `usable` says whether `target` is still plugged in and enabled. `attach`
/// and `detach` do the actual work for one microphone.
///
/// Errors leave the filter on the microphones it was on before, except when
/// letting an old one go fails after the new one took: then it is on both.
pub fn move_filter(
    target: &str,
    attached: &[String],
    usable: bool,
    mut attach: impl FnMut(&str) -> Result<(), String>,
    mut detach: impl FnMut(&str) -> Result<(), String>,
) -> Result<(), String> {
    if !usable {
        return Err(format!("no such microphone: {target}"));
    }
    let was_on_it = attached.iter().any(|a| a.eq_ignore_ascii_case(target));
    if let Err(e) = attach(target) {
        // Half attached is worse than not: clear what was written, but never
        // touch a microphone the filter was on before.
        if !was_on_it {
            let _ = detach(target);
        }
        return Err(e);
    }
    let mut first_error = None;
    for old in attached.iter().filter(|a| !a.eq_ignore_ascii_case(target)) {
        if let Err(e) = detach(old) {
            first_error.get_or_insert(e);
        }
    }
    first_error.map_or(Ok(()), Err)
}

#[cfg(test)]
mod tests {
    use super::*;

    const OLD: &str = "{aaaaaaaa-0000-0000-0000-000000000001}";
    const NEW: &str = "{bbbbbbbb-0000-0000-0000-000000000002}";

    /// Runs a move and returns the result and the calls made, in order.
    fn run(
        attached: &[&str],
        usable: bool,
        attach_fails: bool,
        detach_fails: Option<&str>,
    ) -> (Result<(), String>, Vec<String>) {
        let log = std::cell::RefCell::new(Vec::new());
        let attached: Vec<String> = attached.iter().map(|s| s.to_string()).collect();
        let result = move_filter(
            NEW,
            &attached,
            usable,
            |g| {
                log.borrow_mut().push(format!("attach {g}"));
                if attach_fails {
                    Err("attach failed".into())
                } else {
                    Ok(())
                }
            },
            |g| {
                log.borrow_mut().push(format!("detach {g}"));
                if detach_fails == Some(g) {
                    Err("detach failed".into())
                } else {
                    Ok(())
                }
            },
        );
        (result, log.into_inner())
    }

    #[test]
    fn attaches_the_new_one_before_letting_the_old_one_go() {
        let (r, log) = run(&[OLD], true, false, None);
        assert!(r.is_ok());
        assert_eq!(log, [format!("attach {NEW}"), format!("detach {OLD}")]);
    }

    #[test]
    fn a_microphone_that_is_gone_changes_nothing() {
        let (r, log) = run(&[OLD], false, false, None);
        assert!(r.is_err());
        assert!(log.is_empty());
    }

    #[test]
    fn a_failing_attach_keeps_the_old_microphone() {
        let (r, log) = run(&[OLD], true, true, None);
        assert_eq!(r, Err("attach failed".into()));
        // The old one is never touched; the half-attached new one is cleared.
        assert_eq!(log, [format!("attach {NEW}"), format!("detach {NEW}")]);
    }

    #[test]
    fn a_failing_repair_does_not_detach_the_microphone_it_was_on() {
        let (r, log) = run(&[NEW], true, true, None);
        assert!(r.is_err());
        assert_eq!(log, [format!("attach {NEW}")]);
    }

    #[test]
    fn repairs_in_place() {
        let (r, log) = run(&[NEW], true, false, None);
        assert!(r.is_ok());
        assert_eq!(log, [format!("attach {NEW}")]);
    }

    #[test]
    fn every_old_microphone_is_let_go_even_when_one_fails() {
        let third = "{cccccccc-0000-0000-0000-000000000003}";
        let (r, log) = run(&[OLD, third], true, false, Some(OLD));
        assert_eq!(r, Err("detach failed".into()));
        assert_eq!(
            log,
            [
                format!("attach {NEW}"),
                format!("detach {OLD}"),
                format!("detach {third}")
            ]
        );
    }

    #[test]
    fn guids_compare_regardless_of_case() {
        let (r, log) = run(&[&NEW.to_uppercase()], true, false, None);
        assert!(r.is_ok());
        assert_eq!(log, [format!("attach {NEW}")]);
    }
}
