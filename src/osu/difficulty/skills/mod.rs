use crate::{any::difficulty::skills::StrainSkill, model::mods::GameMods, osu::object::OsuObject};

use self::{aim::Aim, flashlight::Flashlight, memory::Memory, reading::Reading, speed::Speed};

use super::{
    HD_FADE_IN_DURATION_MULTIPLIER, object::OsuDifficultyObject, scaling_factor::ScalingFactor,
};

pub mod aim;
pub mod flashlight;
pub mod memory;
pub mod reading;
pub mod speed;
pub mod strain;

pub struct OsuSkills {
    pub aim: Aim,
    pub aim_no_sliders: Aim,
    pub speed: Speed,
    pub flashlight: Flashlight,
    pub memory: Memory,
    pub reading: Reading,
}

impl OsuSkills {
    pub fn new(
        mods: &GameMods,
        scaling_factor: &ScalingFactor,
        great_hit_window: f64,
        time_preempt: f64,
        approach_rate: f64,
    ) -> Self {
        let hit_window = 2.0 * great_hit_window;

        let time_fade_in = if mods.hd() {
            time_preempt * HD_FADE_IN_DURATION_MULTIPLIER
        } else {
            400.0 * (time_preempt / OsuObject::PREEMPT_MIN).min(1.0)
        };

        // CC V3: pass has_relax flag so the aim skill dispatches to
        // AimRxEvaluator on Relax plays.
        let has_relax = mods.rx();

        let aim = Aim::new(true, has_relax);
        let aim_no_sliders = Aim::new(false, has_relax);
        let speed = Speed::new(hit_window, mods.ap());
        let flashlight = Flashlight::new(mods, scaling_factor.radius, time_preempt, time_fade_in);
        let memory = Memory::new(mods.hd());
        let reading = Reading::new(!mods.rx(), mods.ez(), approach_rate, mods.hd());

        Self {
            aim,
            aim_no_sliders,
            speed,
            flashlight,
            memory,
            reading,
        }
    }

    pub fn process(&mut self, curr: &OsuDifficultyObject<'_>, objects: &[OsuDifficultyObject<'_>]) {
        self.aim.process(curr, objects);
        self.aim_no_sliders.process(curr, objects);
        self.speed.process(curr, objects);
        self.flashlight.process(curr, objects);
        self.memory.process(curr, objects);
        self.reading.process(curr, objects);
    }
}

#[cfg(test)]
mod tests {
    use super::reading::Reading;

    #[test]
    fn reading_skill_stays_off_for_relax() {
        let reading = Reading::new(false, false, 0.0, false);
        assert_eq!(reading.difficulty(), 0.0);
    }
}