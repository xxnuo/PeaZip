use nsh_core::PressEvent;

pub(crate) fn double_click<T: PartialEq>(
    previous: &mut Option<T>,
    target: T,
    event: &PressEvent,
) -> bool {
    // A virtual row can display another item between the two clicks.
    let activate = event.clicks == 2 && previous.as_ref() == Some(&target);
    *previous = (event.clicks == 1).then_some(target);
    activate
}

#[cfg(test)]
mod tests {
    use super::*;
    use nsh_core::keymap::Modifiers;

    #[test]
    fn requires_two_pointer_clicks_on_the_same_snapshot_and_item() {
        let mut previous = None;
        let click = |clicks| PressEvent {
            clicks,
            ..Default::default()
        };
        assert!(!double_click(&mut previous, (1, "a"), &click(2)));
        assert!(!double_click(&mut previous, (1, "a"), &click(1)));
        assert!(!double_click(&mut previous, (1, "b"), &click(2)));
        assert!(!double_click(&mut previous, (1, "a"), &click(1)));
        assert!(!double_click(&mut previous, (2, "a"), &click(2)));
        assert!(!double_click(&mut previous, (2, "a"), &click(1)));
        assert!(double_click(&mut previous, (2, "a"), &click(2)));
        assert!(!double_click(&mut previous, (2, "a"), &click(3)));
        assert!(!double_click(&mut previous, (2, "a"), &click(0)));
        for modifiers in [
            Modifiers {
                shift: true,
                ..Default::default()
            },
            Modifiers {
                ctrl: true,
                ..Default::default()
            },
            Modifiers {
                alt: true,
                ..Default::default()
            },
            Modifiers {
                cmd: true,
                ..Default::default()
            },
        ] {
            assert!(!double_click(&mut previous, (2, "a"), &click(1)));
            assert!(double_click(
                &mut previous,
                (2, "a"),
                &PressEvent {
                    modifiers,
                    ..click(2)
                }
            ));
            assert!(!double_click(&mut previous, (2, "a"), &click(2)));
            assert!(!double_click(
                &mut previous,
                (2, "a"),
                &PressEvent {
                    modifiers,
                    ..click(1)
                }
            ));
            assert!(double_click(&mut previous, (2, "a"), &click(2)));
        }
    }
}
