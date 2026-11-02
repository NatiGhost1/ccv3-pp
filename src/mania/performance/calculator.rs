use crate::{
    GameMods,
    mania::{ManiaDifficultyAttributes, ManiaPerformanceAttributes, ManiaScoreState},
};

pub(super) struct ManiaPerformanceCalculator<'mods> {
    attrs: ManiaDifficultyAttributes,
    mods: &'mods GameMods,
    state: ManiaScoreState,
}

impl<'a> ManiaPerformanceCalculator<'a> {
    pub const fn new(
        attrs: ManiaDifficultyAttributes,
        mods: &'a GameMods,
        state: ManiaScoreState,
    ) -> Self {
        Self { attrs, mods, state }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chordjack_maps_are_nerfed() {
        let attrs = ManiaDifficultyAttributes {
            stars: 7.5,
            n_objects: 300,
            n_hold_notes: 60,
            jack_ratio: 0.5,
            jack_factor: 0.5,
            max_combo: 420,
            is_convert: false,
        };
        let mods = GameMods::default();
        let calc = ManiaPerformanceCalculator::new(attrs.clone(), &mods, ManiaScoreState::default());

        assert!(calc.chordjack_nerf() < 1.0);
    }
}

impl ManiaPerformanceCalculator<'_> {
    fn chordjack_nerf(&self) -> f64 {
        let total_notes = self.attrs.n_objects.max(1);
        let hold_ratio = self.attrs.n_hold_notes as f64 / total_notes as f64;
        let star_pressure = (self.attrs.stars - 5.0).clamp(0.0, 6.0) / 6.0;
        let low_hold_pressure = (0.30 - hold_ratio).clamp(0.0, 0.30) / 0.30;
        let easy_jack_pressure = self.attrs.jack_ratio * (1.0 - self.attrs.jack_factor);

        // Chordjacks are usually dense, note-heavy patterns with unusually few
        // hold notes. They score high star values but are less representative of
        // true technical lock consistency than similar star maps with more
        // natural spacing.
        let intensity = (star_pressure * low_hold_pressure * easy_jack_pressure).clamp(0.0, 1.0);

        1.0 - (intensity * 0.30).clamp(0.0, 0.30)
    }

    pub fn calculate(self) -> ManiaPerformanceAttributes {
        let mut multiplier = 1.0;

        if self.mods.nf() {
            multiplier *= 0.75;
        }

        if self.mods.ez() {
            multiplier *= 0.5;
        }

        multiplier *= self.chordjack_nerf();

        let difficulty_value = self.compute_difficulty_value();
        let pp = difficulty_value * multiplier;

        ManiaPerformanceAttributes {
            difficulty: self.attrs,
            pp,
            pp_difficulty: difficulty_value,
        }
    }

    fn compute_difficulty_value(&self) -> f64 {
        // * Star rating to pp curve
        8.0 * f64::powf(f64::max(self.attrs.stars - 0.15, 0.05), 2.2)
             // * From 80% accuracy, 1/20th of total pp is awarded per additional 1% accuracy
             * self.calculate_custom_accuracy_multiplier()
             // * Length bonus, capped at 1500 notes
             * (1.0 + 0.1 * f64::min(1.0, self.total_hits() / 1500.0))
    }

    const fn total_hits(&self) -> f64 {
        self.state.total_hits() as f64
    }

    fn calculate_custom_accuracy(&self) -> f64 {
        let ManiaScoreState {
            n320,
            n300,
            n200,
            n100,
            n50,
            misses: _,
        } = &self.state;

        let total_hits = self.state.total_hits();

        if total_hits == 0 {
            return 0.0;
        }

        custom_accuracy(*n320, *n300, *n200, *n100, *n50, total_hits)
    }

    fn calculate_custom_accuracy_multiplier(&self) -> f64 {
        let accuracy = self.calculate_custom_accuracy();
        let accuracy = ((accuracy - 0.8) / 0.2).max(0.0);

        if accuracy >= 1.0 {
            1.0
        } else {
            accuracy.powf(1.25)
        }
    }
}

pub(super) fn custom_accuracy(
    n320: u32,
    n300: u32,
    n200: u32,
    n100: u32,
    n50: u32,
    total_hits: u32,
) -> f64 {
    let numerator = n320 * 32 + n300 * 30 + n200 * 20 + n100 * 10 + n50 * 5;
    let denominator = total_hits * 32;

    f64::from(numerator) / f64::from(denominator)
}
