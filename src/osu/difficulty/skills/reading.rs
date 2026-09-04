use crate::{
    any::difficulty::{
        object::{HasStartTime, IDifficultyObject},
        skills::{StrainSkill, strain_decay},
    },
    osu::difficulty::object::OsuDifficultyObject,
};

use super::strain::OsuStrainSkill;

#[derive(Clone)]
pub struct Reading {
    enabled: bool,
    easy: bool,
    hidden: bool,
    approach_rate: f64,
    current_strain: f64,
    strain_peaks: Vec<f64>,
    object_strains: Vec<f64>,
    current_section_peak: f64,
    current_section_end: f64,
}

fn repetition_pressure(deltas: &[f64], jumps: &[f64]) -> f64 {
    if deltas.is_empty() || jumps.is_empty() {
        return 0.0;
    }

    let average_delta = deltas.iter().copied().sum::<f64>() / deltas.len() as f64;
    let average_jump = jumps.iter().copied().sum::<f64>() / jumps.len() as f64;

    let delta_variation = deltas
        .iter()
        .map(|delta| (*delta - average_delta).abs())
        .sum::<f64>()
        / deltas.len() as f64;
    let jump_variation = jumps
        .iter()
        .map(|jump| (*jump - average_jump).abs())
        .sum::<f64>()
        / jumps.len() as f64;

    let delta_stability = 1.0
        - (delta_variation / average_delta.clamp(1.0, f64::INFINITY)).clamp(0.0, 1.0);
    let jump_stability = 1.0
        - (jump_variation / average_jump.clamp(1.0, f64::INFINITY)).clamp(0.0, 1.0);

    let alternating = if deltas.len() > 1 {
        let alternation_count = deltas
            .windows(2)
            .filter(|pair| (pair[0] - pair[1]).abs() > average_delta * 0.08)
            .count() as f64;
        (alternation_count / (deltas.len() - 1) as f64).clamp(0.0, 1.0)
    } else {
        0.0
    };

    ((0.65 * delta_stability + 0.35 * jump_stability) * (0.55 + 0.45 * alternating)).clamp(0.0, 1.0)
}

impl Reading {
    pub fn new(enabled: bool, easy: bool, approach_rate: f64, hidden: bool) -> Self {
        Self {
            enabled,
            easy,
            hidden,
            approach_rate,
            current_strain: 0.0,
            strain_peaks: Vec::with_capacity(128),
            object_strains: Vec::with_capacity(128),
            current_section_peak: 0.0,
            current_section_end: 0.0,
        }
    }

    const SKILL_MULTIPLIER: f64 = 1.75;
    const STRAIN_DECAY_BASE: f64 = 0.15;

    fn calculate_initial_strain(
        &mut self,
        time: f64,
        curr: &OsuDifficultyObject<'_>,
        objects: &[OsuDifficultyObject<'_>],
    ) -> f64 {
        let prev_start_time = curr
            .previous(0, objects)
            .map_or(0.0, HasStartTime::start_time);

        self.current_strain * strain_decay(time - prev_start_time, Self::STRAIN_DECAY_BASE)
    }

    fn strain_value_at(
        &mut self,
        curr: &OsuDifficultyObject<'_>,
        objects: &[OsuDifficultyObject<'_>],
    ) -> f64 {
        self.current_strain *= strain_decay(curr.delta_time, Self::STRAIN_DECAY_BASE);

        let readability = Self::evaluate_reading_diff_of(curr, objects, self.approach_rate, self.hidden);
        self.current_strain += readability * Self::SKILL_MULTIPLIER;

        self.current_strain
    }

    fn evaluate_reading_diff_of(
        curr: &OsuDifficultyObject<'_>,
        objects: &[OsuDifficultyObject<'_>],
        approach_rate: f64,
        hidden: bool,
    ) -> f64 {
        let delta_time = curr.delta_time.max(1.0);
        let density = (1000.0 / delta_time).clamp(0.0, 30.0);
        let bpm = 60_000.0 / delta_time;
        let slow_map = bpm < 200.0;
        let ar = approach_rate.clamp(0.0, 10.0);
        let map_length_factor = (objects.len() as f64 / 300.0).clamp(0.0, 2.2);
        let time_preempt = if approach_rate < 5.0 {
            1800.0 - 120.0 * approach_rate
        } else {
            1200.0 - 150.0 * (approach_rate - 5.0)
        }
        .clamp(450.0, 1800.0);
        let memory_only = Self::all_objects_visible(objects, time_preempt);
        let visible_objects = Self::visible_object_count(curr, objects, time_preempt);
        let screen_density = (visible_objects as f64 / 4.0).clamp(0.75, 3.0);

        let mut pattern_pressure = 0.0;
        let mut speed_variety = 0.0;
        let mut distance_pressure = 0.0;

        if curr.idx > 0 {
            let previous = curr.previous(0, objects).map(|p| p.base).unwrap_or(curr.base);
            let jump = f64::from((curr.base.stacked_pos() - previous.stacked_pos()).length());
            let previous_delta = curr
                .previous(1, objects)
                .map(|p| p.delta_time)
                .unwrap_or(curr.delta_time)
                .max(1.0);
            let rhythm_change = ((curr.delta_time / previous_delta).ln().abs() / 2.0).clamp(0.0, 1.0);
            let angle_pressure = curr.angle.unwrap_or(0.0).abs() / std::f64::consts::PI;
            let jump_speed = jump / delta_time;

            speed_variety = rhythm_change;
            distance_pressure = (jump / 100.0).clamp(0.0, 1.5)
                * (1.0 + (jump_speed / 0.8).clamp(0.0, 1.0));

            pattern_pressure = (jump / 160.0).clamp(0.0, 1.4)
                + rhythm_change * 1.1
                + angle_pressure * 0.9;
        }

        // Higher ARs are easier to read, especially on slower maps where the timing window is not as punishing.
        let high_ar_nerf = if ar >= 8.5 {
            let peak = ((ar - 8.5) / 1.5).clamp(0.0, 1.0);
            let slow_map_bonus = if slow_map { 1.0 } else { 0.7 };
            (1.0 - 0.55 * peak) * slow_map_bonus
        } else {
            1.0
        };

        // Low AR memory is dominated by map length and pattern retention, not raw BPM.
        // Faster maps still get a bit more exposure, but length is the main driver.
        let low_ar_memory = if !hidden && ar < 2.0 {
            let low_ar_factor = (2.0 - ar) / 2.0;
            let speed_pressure = (bpm / 200.0).clamp(0.0, 1.0) * 0.25;
            low_ar_factor * (map_length_factor + 0.3 + speed_pressure)
        } else {
            0.0
        };

        let memory_blend = if memory_only {
            1.0
        } else {
            (low_ar_memory.clamp(0.0, 2.5) / 2.5).powi(2)
        };

        // Keep the easier end of the AR curve from being overstated on 200 BPM and below maps.
        let ar_hardness = if ar < 2.0 {
            let low_ar_factor = (2.0 - ar) / 2.0;
            0.6 * low_ar_factor * if slow_map { 0.2 } else { 1.0 }
        } else if ar >= 8.5 {
            let peak = ((ar - 8.5) / 1.5).clamp(0.0, 1.0);
            let base = 1.0 - peak;
            if slow_map {
                0.25 * base
            } else {
                0.45 * base
            }
        } else {
            0.65 + 0.15 * (ar / 8.5)
        };

        let density_pressure = if memory_only {
            0.0
        } else {
            density * (0.18 + 0.72 * ar_hardness)
        };
        let pattern_factor = pattern_pressure * (0.75 + 0.9 * memory_blend + 0.45 * ar_hardness);

        let local_repeat_pressure = {
            let lookback = (curr.idx.saturating_sub(8)).min(objects.len().saturating_sub(1));
            let recent_deltas: Vec<f64> = (0..=lookback)
                .filter_map(|offset| {
                    let obj = objects.get(curr.idx.saturating_sub(offset))?;
                    if obj.idx == curr.idx {
                        return None;
                    }
                    Some(obj.delta_time.max(1.0) as f64)
                })
                .collect();
            let recent_jumps: Vec<f64> = (0..=lookback)
                .filter_map(|offset| {
                    let obj = objects.get(curr.idx.saturating_sub(offset))?;
                    if obj.idx == curr.idx {
                        return None;
                    }
                    let prev = objects.get(curr.idx.saturating_sub(offset.saturating_add(1)))?;
                    let jump = f64::from((obj.base.stacked_pos() - prev.base.stacked_pos()).length());
                    Some(jump)
                })
                .collect();

            repetition_pressure(&recent_deltas, &recent_jumps)
        };

        let mut readability = (density_pressure + pattern_factor)
            * screen_density
            * (1.0 + speed_variety * 0.8 + distance_pressure * 0.35)
            * (1.0 + 0.65 * memory_blend)
            * high_ar_nerf;

        readability *= 1.0 - (local_repeat_pressure * 0.9);

        let mut previous_delta = delta_time;
        for i in 0..4 {
            let Some(previous) = curr.previous(i, objects) else {
                break;
            };

            speed_variety += ((previous.delta_time.max(1.0) / previous_delta).ln().abs() / 2.0)
                .clamp(0.0, 1.0);
            previous_delta = previous.delta_time.max(1.0);
        }
        speed_variety = (speed_variety / 5.0).clamp(0.0, 1.0);

        let ar_hardness = (8.0 - approach_rate.clamp(0.0, 10.0)).clamp(0.0, 8.0) / 8.0;
        let memory_blend = if memory_only {
            1.0
        } else {
            (1.0 - (approach_rate / 7.0).clamp(0.0, 1.0)).powi(2)
        };
        let density_pressure = if memory_only {
            0.0
        } else {
            density * (0.35 + 0.65 * ar_hardness)
        };
        let pattern_factor = pattern_pressure * (0.9 + 0.8 * memory_blend);
        let screen_pattern_factor = screen_density
            * (1.0 + speed_variety * 0.8 + distance_pressure * 0.35);

        let mut readability = (density_pressure + pattern_factor) * screen_pattern_factor
            * (1.0 + memory_blend * 0.7);

        if hidden {
            readability *= 1.08;
        }

        readability / 6.0
    }

    fn visible_object_count(
        curr: &OsuDifficultyObject<'_>,
        objects: &[OsuDifficultyObject<'_>],
        time_preempt: f64,
    ) -> usize {
        let mut count = 1;

        for i in 0..curr.idx {
            let Some(previous) = curr.previous(i, objects) else {
                break;
            };

            if curr.start_time - previous.start_time > time_preempt {
                break;
            }

            count += 1;
        }

        count
    }

    fn all_objects_visible(objects: &[OsuDifficultyObject<'_>], time_preempt: f64) -> bool {
        let Some(first) = objects.first() else {
            return false;
        };
        let Some(last) = objects.last() else {
            return false;
        };

        last.start_time - first.start_time <= time_preempt
    }

    pub fn difficulty(&self) -> f64 {
        if !self.enabled {
            return 0.0;
        }

        let peaks = self.strain_peaks.clone();
        let mut difficulty = 0.0;
        let mut weight = 1.0;

        for peak in peaks.iter().filter(|p| **p > 0.0).copied().collect::<Vec<_>>() {
            difficulty += peak * weight;
            weight *= 0.9;
        }

        if self.easy {
            difficulty *= 0.72;
        }

        difficulty
    }
}

impl StrainSkill for Reading {
    type DifficultyObject<'a> = OsuDifficultyObject<'a>;
    type DifficultyObjects<'a> = [OsuDifficultyObject<'a>];

    fn process<'a>(
        &mut self,
        curr: &Self::DifficultyObject<'a>,
        objects: &Self::DifficultyObjects<'a>,
    ) {
        if !self.enabled {
            return;
        }

        let section_length = f64::from(Self::SECTION_LENGTH);

        if curr.idx == 0 {
            self.current_section_end = f64::ceil(curr.start_time / section_length) * section_length;
        }

        while curr.start_time > self.current_section_end {
            self.strain_peaks.push(self.current_section_peak);
            self.current_section_peak = self.calculate_initial_strain(self.current_section_end, curr, objects);
            self.current_section_end += section_length;
        }

        let strain = self.strain_value_at(curr, objects);
        self.current_section_peak = f64::max(strain, self.current_section_peak);
        self.object_strains.push(strain);
    }

    fn object_strains(&self) -> &[f64] {
        &self.object_strains
    }

    fn count_top_weighted_strains(&self, difficulty_value: f64) -> f64 {
        crate::any::difficulty::skills::count_top_weighted_strains(&self.object_strains, difficulty_value)
    }

    fn save_current_peak(&mut self) {
        self.strain_peaks.push(self.current_section_peak);
    }

    fn start_new_section_from<'a>(
        &mut self,
        time: f64,
        curr: &Self::DifficultyObject<'a>,
        objects: &Self::DifficultyObjects<'a>,
    ) {
        self.current_section_peak = self.calculate_initial_strain(time, curr, objects);
    }

    fn into_current_strain_peaks(self) -> Vec<f64> {
        let mut peaks = self.strain_peaks;
        peaks.push(self.current_section_peak);
        peaks
    }

    fn difficulty_value(current_strain_peaks: Vec<f64>) -> f64 {
        crate::any::difficulty::skills::difficulty_value(current_strain_peaks, Self::DECAY_WEIGHT)
    }

    fn into_difficulty_value(self) -> f64 {
        Self::difficulty_value(self.into_current_strain_peaks())
    }

    fn cloned_difficulty_value(&self) -> f64 {
        Self::difficulty_value(self.clone().into_current_strain_peaks())
    }
}

impl OsuStrainSkill for Reading {}

#[cfg(test)]
mod tests {
    use rosu_map::section::general::GameMode;

    use crate::{Beatmap, Difficulty, osu::Osu};

    #[test]
    fn repetitive_1_2_patterns_have_low_reading_pressure() {
        let deltas = [180.0, 120.0, 180.0, 120.0, 180.0, 120.0, 180.0, 120.0];
        let jumps = [60.0, 80.0, 60.0, 80.0, 60.0, 80.0, 60.0, 80.0];

        let suppression = super::repetition_pressure(&deltas, &jumps);
        assert!(suppression > 0.7, "repeating 1-2 patterns should be heavily suppressed");
    }

    #[test]
    fn reading_skill_tracks_map_ar() {
        let map = Beatmap::from_path("./resources/nati-1.osu").unwrap();
        assert_eq!(map.mode, GameMode::Osu);

        let low_ar = Difficulty::new()
            .ar(5.0, false)
            .calculate_for_mode::<Osu>(&map)
            .unwrap();
        let high_ar = Difficulty::new()
            .ar(9.8, false)
            .calculate_for_mode::<Osu>(&map)
            .unwrap();

        assert_ne!(low_ar.reading, high_ar.reading);
        assert!(low_ar.reading > high_ar.reading, "low AR should be harder to read than high AR");
    }
}
