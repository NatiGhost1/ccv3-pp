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

        let mut pattern_pressure = 0.0;

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

            pattern_pressure = (jump / 160.0).clamp(0.0, 1.4)
                + rhythm_change * 1.1
                + angle_pressure * 0.9;
        }

        let ar_hardness = (8.0 - approach_rate.clamp(0.0, 10.0)).clamp(0.0, 8.0) / 8.0;
        let memory_blend = (1.0 - (approach_rate / 7.0).clamp(0.0, 1.0)).powi(2);
        let density_pressure = density * (0.35 + 0.65 * ar_hardness);
        let pattern_factor = pattern_pressure * (0.9 + 0.8 * memory_blend);

        let mut readability = (density_pressure + pattern_factor) * (1.0 + memory_blend * 0.7);

        if hidden {
            readability *= 1.12;
        }

        readability / 6.0
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
