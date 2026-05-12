use std::{env, fmt::Display, path::PathBuf, process::exit, time::SystemTime};

use rosu_mods::GameModsLegacy;
use ccv3_pp::{Beatmap, Difficulty, GameMods, Performance};

const DEFAULT_PLAYS: usize = 5;

struct SimpleRng(u64);

impl SimpleRng {
    fn new(seed: u64) -> Self {
        Self(seed)
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    fn gen_range(&mut self, lower: u32, upper: u32) -> u32 {
        if lower >= upper {
            lower
        } else {
            let range = (upper - lower) as u64;
            let value = self.next_u64();
            lower + (value % range) as u32
        }
    }

    fn choose<'a, T>(&mut self, values: &'a [T]) -> &'a T {
        let index = self.gen_range(0, values.len() as u32) as usize;
        &values[index]
    }

    fn choose_optional<'a, T>(&mut self, values: &'a [T]) -> Option<&'a T> {
        if self.gen_range(0, 2) == 0 {
            None
        } else {
            Some(self.choose(values))
        }
    }
}

#[derive(Clone, Copy)]
struct ModOption {
    name: &'static str,
    mod_value: GameModsLegacy,
}

impl Display for ModOption {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name)
    }
}

const SPEED_MODS: &[ModOption] = &[
    ModOption {
        name: "NoMod",
        mod_value: GameModsLegacy::NoMod,
    },
    ModOption {
        name: "DoubleTime",
        mod_value: GameModsLegacy::DoubleTime,
    },
    ModOption {
        name: "Nightcore",
        mod_value: GameModsLegacy::Nightcore,
    },
    ModOption {
        name: "HalfTime",
        mod_value: GameModsLegacy::HalfTime,
    },
];

const DIFFICULTY_MODS: &[ModOption] = &[
    ModOption {
        name: "Easy",
        mod_value: GameModsLegacy::Easy,
    },
    ModOption {
        name: "HardRock",
        mod_value: GameModsLegacy::HardRock,
    },
];

const VISUAL_MODS: &[ModOption] = &[
    ModOption {
        name: "Hidden",
        mod_value: GameModsLegacy::Hidden,
    },
    ModOption {
        name: "Flashlight",
        mod_value: GameModsLegacy::Flashlight,
    },
    ModOption {
        name: "Mirror",
        mod_value: GameModsLegacy::Mirror,
    },
];

const UTILITY_MODS: &[ModOption] = &[
    ModOption {
        name: "NoFail",
        mod_value: GameModsLegacy::NoFail,
    },
    ModOption {
        name: "SpunOut",
        mod_value: GameModsLegacy::SpunOut,
    },
];

const SPECIAL_MODS: &[ModOption] = &[
    ModOption {
        name: "Relax",
        mod_value: GameModsLegacy::Relax,
    },
    ModOption {
        name: "Autopilot",
        mod_value: GameModsLegacy::Autopilot,
    },
];

const ENDURANCE_MODS: &[ModOption] = &[
    ModOption {
        name: "SuddenDeath",
        mod_value: GameModsLegacy::SuddenDeath,
    },
    ModOption {
        name: "Perfect",
        mod_value: GameModsLegacy::Perfect,
    },
];

fn main() {
    let args: Vec<String> = env::args().collect();

    if args.len() < 2 {
        eprintln!(
            "Usage: {} <beatmap.osu> [plays] [seed]\nExample: {} ./resources/2785319.osu 8 12345",
            args[0], args[0]
        );
        exit(1);
    }

    let map_path = PathBuf::from(&args[1]);
    let plays = args
        .get(2)
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(DEFAULT_PLAYS);
    let seed = args
        .get(3)
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or_else(|| {
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos() as u64
        });

    let mut rng = SimpleRng::new(seed);
    let map = match Beatmap::from_path(&map_path) {
        Ok(map) => map,
        Err(err) => {
            eprintln!("Failed to parse beatmap '{}': {err}", map_path.display());
            exit(1);
        }
    };

    println!("Beatmap: {}", map_path.display());
    println!("Mode: {:?}", map.mode);
    println!("BPM: {:.2}", map.bpm());
    println!("Total hitobjects: {}", map.hit_objects.len());
    println!("Seed: {seed}");
    println!("Generating {plays} random plays...\n");

    for index in 1..=plays {
        let (mod_list, mods_legacy) = build_random_mod_combo(&mut rng);
        let mods = GameMods::from(mods_legacy);

        let diff_attrs = Difficulty::new().mods(mods.clone()).calculate(&map);
        let max_combo = diff_attrs.max_combo();
        let misses = rng.gen_range(0, (max_combo / 15).max(1) + 1);
        let combo = if misses == 0 {
            max_combo
        } else {
            let lower_combo = (max_combo / 2).max(1);
            rng.gen_range(lower_combo, max_combo + 1)
        };
        let accuracy = 90.0 + (rng.gen_range(0, 1001) as f64 / 100.0);

        let perf_attrs = Performance::new(diff_attrs)
            .mods(mods)
            .combo(combo)
            .misses(misses)
            .accuracy(accuracy)
            .calculate();

        println!("Play #{index}");
        println!("  Mods: {mod_list}");
        println!("  Stars: {:.2}", perf_attrs.stars());
        println!("  PP: {:.2}", perf_attrs.pp());
        println!("  Combo: {combo}/{max_combo}");
        println!("  Misses: {misses}");
        println!("  Accuracy: {accuracy:.2}%\n");
    }
}

fn build_random_mod_combo(rng: &mut SimpleRng) -> (String, GameModsLegacy) {
    let speed = rng.choose(SPEED_MODS);
    let difficulty = rng.choose_optional(DIFFICULTY_MODS);
    let visual = rng.choose_optional(VISUAL_MODS);
    let utility = rng.choose_optional(UTILITY_MODS);
    let special = rng.choose_optional(SPECIAL_MODS);
    let endurance = rng.choose_optional(ENDURANCE_MODS);

    let mut mods = speed.mod_value;
    let mut names = vec![speed.name.to_owned()];

    if let Some(difficulty) = difficulty {
        mods |= difficulty.mod_value;
        names.push(difficulty.name.to_owned());
    }

    if let Some(visual) = visual {
        mods |= visual.mod_value;
        names.push(visual.name.to_owned());
    }

    if let Some(utility) = utility {
        mods |= utility.mod_value;
        names.push(utility.name.to_owned());
    }

    if let Some(special) = special {
        if special.mod_value != GameModsLegacy::Autopilot || !mods.contains(GameModsLegacy::Relax) {
            mods |= special.mod_value;
            names.push(special.name.to_owned());
        }
    }

    if let Some(endurance) = endurance {
        if endurance.mod_value != GameModsLegacy::Perfect
            || !mods.contains(GameModsLegacy::SuddenDeath)
        {
            mods |= endurance.mod_value;
            names.push(endurance.name.to_owned());
        }
    }

    if names.len() == 1 && names[0] == "NoMod" {
        names[0] = "NoMod".to_owned();
    }

    let mod_list = if names.iter().all(|name| name == "NoMod") {
        "NoMod".to_string()
    } else {
        names
            .into_iter()
            .filter(|name| name != "NoMod")
            .collect::<Vec<String>>()
            .join(" + ")
    };

    (mod_list, mods)
}
