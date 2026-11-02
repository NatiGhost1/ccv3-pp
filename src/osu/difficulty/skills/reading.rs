use rosu_map::util::Pos;

use crate::{
    any::difficulty::{
        object::{HasStartTime, IDifficultyObject},
        skills::{StrainSkill, strain_decay},
    },
    osu::{difficulty::object::OsuDifficultyObject, object::OsuObject},
};

fn preempt_for_ar(approach_rate: f64) -> f64 {
    // Convert AR to the visibility window used to count objects on screen.
    let preempt = if approach_rate < 5.0 {
        1200.0 - 120.0 * approach_rate
    } else {
        1200.0 - 150.0 * (approach_rate - 5.0)
    };

    preempt.clamp(OsuObject::PREEMPT_MIN, 1200.0)
}

define_skill! {
    #[derive(Clone)]
    pub struct Reading: StrainSkill => [OsuDifficultyObject<'a>][OsuDifficultyObject<'a>] {
        current_strain: f64 = 0.0,
        enabled: bool = false,
        preempt: f64 = OsuObject::PREEMPT_MIN,
        hidden: bool = false,
        easy: bool = false,
        density_end: usize = 0,
        previous_delta_time: Option<f64> = None,
        previous_position: Option<Pos> = None,
        previous_previous_position: Option<Pos> = None,
    }
}

impl Reading {
    // Keep reading subtle relative to aim/speed, then share the same scale for stars and PP.
    const SKILL_MULTIPLIER: f64 = 0.08;
    const STRAIN_DECAY_BASE: f64 = 0.3;
    const DIFFICULTY_TO_RATING: f64 = 0.09;

    pub fn difficulty_to_rating(difficulty: f64) -> f64 {
        difficulty * Self::DIFFICULTY_TO_RATING
    }

    // `enabled` disables the skill for unsupported plays; EZ and HD adjust its contribution.
    pub fn with_settings(enabled: bool, easy: bool, approach_rate: f64, hidden: bool) -> Self {
        Self {
            enabled,
            preempt: preempt_for_ar(approach_rate),
            hidden,
            easy,
            ..Self::new()
        }
    }

    fn calculate_initial_strain(
        &mut self,
        time: f64,
        curr: &OsuDifficultyObject<'_>,
        objects: &[OsuDifficultyObject<'_>],
    ) -> f64 {
        let previous_start_time = curr
            .previous(0, objects)
            .map_or(0.0, HasStartTime::start_time);

        // Preserve the accumulated reading strain when a new strain section begins.
        self.current_strain * strain_decay(time - previous_start_time, Self::STRAIN_DECAY_BASE)
    }

    fn strain_value_at(
        &mut self,
        curr: &OsuDifficultyObject<'_>,
        objects: &[OsuDifficultyObject<'_>],
    ) -> f64 {
        self.current_strain *= strain_decay(curr.delta_time, Self::STRAIN_DECAY_BASE);

        if !self.enabled {
            return self.current_strain;
        }

        // Keep a forward-only end index: each future object is examined once as the map is processed.
        let density_limit = curr.start_time + self.preempt;
        self.density_end = self.density_end.max(curr.idx);

        while self.density_end < objects.len()
            && objects[self.density_end].start_time <= density_limit
        {
            self.density_end += 1;
        }

        // Count the current object and upcoming objects visible within this object's preempt window.
        let on_screen = self.density_end.saturating_sub(curr.idx).max(1);
        // Ignore the first two visible objects, then rise linearly to a cap at ten.
        let density = ((on_screen as f64 - 2.0) / 8.0).clamp(0.0, 1.0);
        let position = curr.base.stacked_pos();
        let novelty = self.transition_novelty(position, curr.delta_time);
        let mod_factor = if self.easy {
            0.75
        } else if self.hidden {
            1.1
        } else {
            1.0
        };

        // Dense screens are harder to read when timing or movement changes, but mods temper the effect.
        self.current_strain += density * novelty * mod_factor * Self::SKILL_MULTIPLIER;
        self.previous_previous_position = self.previous_position;
        self.previous_position = Some(position);
        self.previous_delta_time = Some(curr.delta_time);

        self.current_strain
    }

    fn transition_novelty(&self, position: Pos, delta_time: f64) -> f64 {
        let (Some(previous_position), Some(previous_delta_time)) =
            (self.previous_position, self.previous_delta_time)
        else {
            return 1.0;
        };

        let rhythm_change = (delta_time.max(1.0) / previous_delta_time.max(1.0))
            .ln()
            .abs()
            .clamp(0.0, 1.0);

        let mut repetition = 0.0;

        // Similar movement axes, distances, and rhythms identify repeated transitions.
        if let Some(previous_previous_position) = self.previous_previous_position {
            let previous_vector = previous_position - previous_previous_position;
            let current_vector = position - previous_position;
            let previous_length = f64::from(previous_vector.length());
            let current_length = f64::from(current_vector.length());

            if previous_length > 0.0 && current_length > 0.0 {
                let direction_similarity = (f64::from(previous_vector.dot(current_vector))
                    / (previous_length * current_length))
                    .abs()
                    .clamp(0.0, 1.0);
                let distance_similarity = (-(previous_length / current_length).ln().abs())
                    .exp()
                    .clamp(0.0, 1.0);
                let rhythm_similarity = (-rhythm_change).exp();

                repetition = direction_similarity * distance_similarity * rhythm_similarity;
            }
        }

        // Novelty stays bounded; repetition can discount it by at most 65%.
        let spatial_change = (f64::from((position - previous_position).length()) / 80.0)
            .clamp(0.0, 1.0);
        let novelty = (0.5 + 0.25 * rhythm_change + 0.25 * spatial_change).clamp(0.5, 1.0);

        novelty * (1.0 - 0.65 * repetition)
    }

    pub fn difficulty(&self) -> f64 {
        self.cloned_difficulty_value()
    }

    pub fn performance_value(
        difficulty: f64,
        accuracy: f64,
        combo: f64,
        max_combo: f64,
        misses: f64,
        total_hits: f64,
    ) -> f64 {
        // Convert through the reading rating scale before applying the PP curve.
        let base = 20.0 * Self::difficulty_to_rating(difficulty).powf(2.0);
        let accuracy_factor = (0.4 + 0.6 * accuracy.clamp(0.0, 1.0)).powf(1.5);
        let combo_factor = (0.5 + 0.5 * (combo / max_combo.max(1.0)).clamp(0.0, 1.0)).powf(0.8);
        let miss_factor = if misses > 0.0 {
            0.97 * (1.0 - (misses / total_hits.max(1.0)).clamp(0.0, 1.0)).powf(0.8)
        } else {
            1.0
        };

        base * accuracy_factor * combo_factor * miss_factor
    }
}


#[cfg(test)]
mod tests {
    use super::Reading;

    #[test]
    fn disabled_reading_has_no_difficulty() {
        assert_eq!(Reading::with_settings(false, false, 9.0, false).difficulty(), 0.0);
    }

    #[test]
    fn reading_performance_scales_with_play_quality() {
        let perfect = Reading::performance_value(1.0, 1.0, 1000.0, 1000.0, 0.0, 1000.0);
        let imperfect = Reading::performance_value(1.0, 0.95, 800.0, 1000.0, 2.0, 1000.0);

        assert!(perfect > imperfect);
    }

    #[test]
    fn reading_performance_uses_the_rating_scale() {
        let value = Reading::performance_value(5.0, 1.0, 1000.0, 1000.0, 0.0, 1000.0);

        assert!((value - 4.05).abs() < 1e-10);
    }
}