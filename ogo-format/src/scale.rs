//! Scaling a picture up for the screen.

/// For each of `shown` screen pixels in a row (or column), the one or two of `size` picture pixels
/// it shows: the nearest, or both neighbours when it falls about halfway between them, to be
/// averaged. At 1.5x that makes every third pixel a blend instead of doubling every other one.
pub fn steps(size: u32, shown: u32) -> Vec<(usize, usize)> {
    let last = size.saturating_sub(1) as usize;
    (0..shown)
        .map(|i| {
            // The screen pixel's centre, in picture pixels
            let position = (2 * i + 1) as f32 * size as f32 / (2 * shown) as f32 - 0.5;
            if position <= 0.0 {
                return (0, 0);
            }
            let below = position.floor() as usize;
            let fraction = position - position.floor();
            let (a, b) = if fraction <= 0.25 {
                (below, below)
            } else if fraction >= 0.75 {
                (below + 1, below + 1)
            } else {
                (below, below + 1)
            };
            (a.min(last), b.min(last))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_and_a_half() {
        assert_eq!(
            steps(4, 6),
            [(0, 0), (0, 1), (1, 1), (2, 2), (2, 3), (3, 3)]
        );
        let steps = steps(160, 240);
        assert_eq!(steps.len(), 240);
        assert_eq!(steps[239], (159, 159));
    }

    #[test]
    fn same_size_and_double() {
        assert_eq!(steps(3, 3), [(0, 0), (1, 1), (2, 2)]);
        assert_eq!(steps(2, 4), [(0, 0), (0, 0), (1, 1), (1, 1)]);
    }
}
