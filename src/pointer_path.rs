use std::time::Duration;

// Issue #323's GTK reproduction succeeds with 20 motions 30 ms apart. Keep
// that measured pacing across XTEST, uinput and portal drags.
pub(crate) const DRAG_STEPS: i64 = 20;
pub(crate) const DRAG_STEP_DELAY: Duration = Duration::from_millis(30);

pub(crate) fn drag_points(start: (i32, i32), end: (i32, i32)) -> impl Iterator<Item = (i32, i32)> {
    (1..=DRAG_STEPS).map(move |step| {
        let interpolate = |from: i32, to: i32| {
            (i64::from(from) + (i64::from(to) - i64::from(from)) * step / DRAG_STEPS) as i32
        };
        (interpolate(start.0, end.0), interpolate(start.1, end.1))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drag_path_reaches_the_endpoint_without_overflow_or_overshoot() {
        for (start, end) in [
            ((0, 0), (500, 300)),
            ((10, 20), (-300, -900)),
            ((i32::MIN, i32::MAX), (i32::MAX, i32::MIN)),
            ((1, 1), (1, 1)),
        ] {
            let points: Vec<_> = drag_points(start, end).collect();
            assert_eq!(points.last(), Some(&end));
            assert!(points.len() > 1);
            for (x, y) in points {
                assert!((start.0.min(end.0)..=start.0.max(end.0)).contains(&x));
                assert!((start.1.min(end.1)..=start.1.max(end.1)).contains(&y));
            }
        }
    }
}
