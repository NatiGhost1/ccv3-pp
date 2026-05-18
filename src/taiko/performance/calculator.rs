use crate::{
    GameMods,
    taiko::{TaikoDifficultyAttributes, TaikoPerformanceAttributes, TaikoScoreState},
    util::{
        difficulty::{erf, erf_inv, logistic, reverse_lerp},
        float_ext::FloatExt,
    },
};

pub(super) struct TaikoPerformanceCalculator<'mods> {
    attrs: TaikoDifficultyAttributes,
    mods: &'mods GameMods,
    state: TaikoScoreState,
    is_classic: bool,
}

impl<'a> TaikoPerformanceCalculator<'a> {
    pub const fn new(
        attrs: TaikoDifficultyAttributes,
        mods: &'a GameMods,
        state: TaikoScoreState,
        is_classic: bool,
    ) -> Self {
        Self {
            attrs,
            mods,
            state,
            is_classic,
        }
    }
}

impl TaikoPerformanceCalculator<'_> {
    pub fn calculate(self) -> TaikoPerformanceAttributes {
        let estimated_unstable_rate =
            if self.state.hitresults.n300 == 0 || self.attrs.great_hit_window <= 0.0 {
                None
            } else {
                Some(
                    self.compute_deviation_upper_bound(
                        f64::from(self.state.hitresults.n300) / self.total_hits(),
                    ) * 10.0,
                )
            };

        let total_difficult_hits = self.total_hits() * self.attrs.consistency_factor;

        let difficulty_value =
            self.compute_difficulty_value(total_difficult_hits, estimated_unstable_rate) * 1.08;
        let accuracy_value =
            self.compute_accuracy_value(total_difficult_hits, estimated_unstable_rate) * 1.1;

        let pp = difficulty_value + accuracy_value;

        TaikoPerformanceAttributes {
            difficulty: self.attrs,
            pp,
            pp_acc: accuracy_value,
            pp_difficulty: difficulty_value,
            estimated_unstable_rate,
        }
    }

    // Tapping Speed Analysis
    fn estimate_average_effective_bpm(&self) -> f64 {
        // Best available proxy using difficulty attributes
        // mono_stamina_factor correlates strongly with required tapping speed
        let base_speed = self.attrs.mono_stamina_factor * 8.5; // rough conversion to BPM-ish scale
        base_speed.clamp(80.0, 320.0)
    }

    fn estimate_peak_tapping_bpm(&self) -> f64 {
        // Assume peak is noticeably higher than average
        self.estimate_average_effective_bpm() * 1.35
    }

    fn is_likely_unlucky_break_miss(&self) -> bool {
        let misses = self.state.hitresults.misses;
        let total_hits = self.total_hits();
        if total_hits == 0.0 || misses == 0 {
            return false;
        }

        let accuracy = f64::from(self.state.hitresults.n300) / total_hits;
        let avg_bpm = self.estimate_average_effective_bpm();

        // - Few misses
        // - High overall accuracy (good player)
        // - Map has relatively low average tapping speed (more likely to have slower sections/breaks)
        misses <= 3
            && accuracy >= 0.94
            && avg_bpm < 210.0 // Significantly slower than peak speed maps
    }

    fn compute_difficulty_value(
        &self,
        total_difficult_hits: f64,
        estimated_unstable_rate: Option<f64>,
    ) -> f64 {
        let Some(estimated_unstable_rate) = estimated_unstable_rate else {
            return 0.0;
        };

        if FloatExt::eq(total_difficult_hits, 0.0) {
            return 0.0;
        }

        let attrs = &self.attrs;

        let rhythm_expected_unstable_rate = self.compute_deviation_upper_bound(1.0) * 10.0;
        let rhythm_maximum_unstable_rate = self.compute_deviation_upper_bound(0.8) * 10.0;

        let rhythm_factor = reverse_lerp(attrs.rhythm / attrs.stars, 0.15, 0.4);

        let rhythm_penalty = 1.0
            - logistic(
                estimated_unstable_rate,
                (rhythm_expected_unstable_rate + rhythm_maximum_unstable_rate) / 2.0,
                10.0 / (rhythm_maximum_unstable_rate - rhythm_expected_unstable_rate),
                Some(0.25 * f64::powf(rhythm_factor, 3.0)),
            );

        let base_difficulty = 5.0 * f64::max(1.0, attrs.stars * rhythm_penalty / 0.11) - 4.0;

        let mut difficulty_value = f64::min(
            f64::powf(base_difficulty, 3.0) / 69052.51,
            f64::powf(base_difficulty, 2.25) / 1250.0,
        );

        difficulty_value *= 1.0 + 0.10 * f64::max(0.0, self.attrs.stars - 10.0);

        let length_bonus = 1.0 + 0.25 * total_difficult_hits / (total_difficult_hits + 4000.0);
        difficulty_value *= length_bonus;

        // Unlucky miss on slower section
        let miss_penalty_base = if self.is_likely_unlucky_break_miss() {
            0.9995 // Extremely soft penalty — unlucky miss barely hurts PP
        } else {
            0.97 + 0.03 * total_difficult_hits / (total_difficult_hits + 1500.0)
        };

        difficulty_value *= f64::powf(miss_penalty_base, f64::from(self.state.hitresults.misses));

        if self.mods.hd() {
            let mut hidden_bonus = if self.attrs.is_convert { 0.025 } else { 0.1 };

            if !self.mods.fl() {
                if !self.is_classic {
                    hidden_bonus *= 0.2;
                }
                if self.mods.ez() && self.is_classic {
                    hidden_bonus *= 0.5;
                }
            }

            difficulty_value *= 1.0 + hidden_bonus;
        }

        if self.mods.fl() {
            difficulty_value *= f64::max(
                1.0,
                1.05 - f64::min(self.attrs.mono_stamina_factor / 50.0, 1.0) * length_bonus,
            );
        }

        let mono_acc_scaling_exponent = f64::from(2) + self.attrs.mono_stamina_factor;
        let mono_acc_scaling_shift =
            f64::from(500) - f64::from(100) * (self.attrs.mono_stamina_factor * f64::from(3));

        difficulty_value
            * (erf(mono_acc_scaling_shift / (f64::sqrt(2.0) * estimated_unstable_rate)))
                .powf(mono_acc_scaling_exponent)
    }

    // compute_accuracy_value, compute_deviation_upper_bound, and total_hits remain unchanged
    fn compute_accuracy_value(
        &self,
        total_difficult_hits: f64,
        estimated_unstable_rate: Option<f64>,
    ) -> f64 {
        let Some(estimated_unstable_rate) = estimated_unstable_rate else {
            return 0.0;
        };

        if self.attrs.great_hit_window <= 0.0 {
            return 0.0;
        }

        let mut accuracy_value = 470.0 * f64::powf(0.9885, estimated_unstable_rate);

        accuracy_value *= 1.0
            + f64::powf(50.0 / estimated_unstable_rate, 2.0) * f64::powf(self.attrs.stars, 2.8)
                / 600.0;

        if self.mods.hd() && !self.attrs.is_convert {
            accuracy_value *= 1.075;
        }

        accuracy_value *= 1.0 + 0.3 * total_difficult_hits / (total_difficult_hits + 4000.0);

        let memory_length_bonus = f64::min(1.15, f64::powf(self.total_hits() / 1500.0, 0.3));

        if self.mods.fl() && self.mods.hd() && !self.attrs.is_convert {
            accuracy_value *= f64::max(1.0, 1.05 * memory_length_bonus);
        }

        accuracy_value
    }

    fn compute_deviation_upper_bound(&self, accuracy: f64) -> f64 {
        #[expect(clippy::unreadable_literal, reason = "staying in-sync with lazer")]
        const Z: f64 = 2.32634787404;

        let n = self.total_hits();
        let p = accuracy;

        let p_lower_bound = (n * p + Z * Z / 2.0) / (n + Z * Z)
            - Z / (n + Z * Z) * f64::sqrt(n * p * (1.0 - p) + Z * Z / 4.0);

        self.attrs.great_hit_window / (f64::sqrt(2.0) * erf_inv(p_lower_bound))
    }

    const fn total_hits(&self) -> f64 {
        self.state.hitresults.total_hits() as f64
    }
}