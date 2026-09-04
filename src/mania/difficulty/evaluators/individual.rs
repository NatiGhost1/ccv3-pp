use crate::{mania::difficulty::object::ManiaDifficultyObject, util::sync::Weak};

pub struct IndividualStrainEvaluator;

impl IndividualStrainEvaluator {
    pub fn evaluate_diff_of(curr: &ManiaDifficultyObject) -> f64 {
        let mania_curr = curr;
        let start_time = curr.start_time;
        let end_time = curr.end_time;

        // * We award a bonus if this note starts and ends before the end of another hold note.
        let with_bonus = mania_curr
            .prev_hit_objects
            .iter()
            .flatten()
            .filter_map(Weak::upgrade)
            .any(|rc| {
                let mania_prev = rc.get();

                mania_prev.end_time > end_time + 1.0 && start_time > mania_prev.start_time + 1.0
            });

        // * Factor to all additional strains in case something else is held
        let hold_factor = if with_bonus { 1.25 } else { 1.0 };

        (2.0 + 3.0 * Self::jack_factor(curr)) * hold_factor
    }

    pub fn is_jack(curr: &ManiaDifficultyObject) -> bool {
        curr.prev_hit_objects[curr.column]
            .as_ref()
            .and_then(Weak::upgrade)
            .is_some_and(|prev| curr.start_time - prev.get().start_time <= 180.0)
    }

    pub fn jack_factor(curr: &ManiaDifficultyObject) -> f64 {
        let Some(prev) = curr.prev_hit_objects[curr.column]
            .as_ref()
            .and_then(Weak::upgrade)
        else {
            return 0.0;
        };

        let interval = curr.start_time - prev.get().start_time;

        if interval > 180.0 {
            return 0.0;
        }

        f64::exp(-interval / 180.0)
    }
}
