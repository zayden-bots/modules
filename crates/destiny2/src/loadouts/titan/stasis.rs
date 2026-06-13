use std::fmt;
use std::fmt::{Display, Formatter};

use super::super::{
    Abilities as AbilitiesTrait,
    Aspect as AspectTrait,
    StasisFragment,
    StasisGrenade,
    box_display,
};
use super::{ClassAbility, Jump};

#[derive(Clone, Copy)]
pub(crate) struct Abilities {
    pub super_: Super,
    pub class: ClassAbility,
    pub jump: Jump,
    pub melee: Melee,
    pub grenade: StasisGrenade,
}

impl AbilitiesTrait for Abilities {
    fn super_(&self) -> Box<dyn Display> {
        Box::new(self.super_)
    }

    fn class(&self) -> Box<dyn Display> {
        Box::new(self.class)
    }

    fn jump(&self) -> Box<dyn Display> {
        Box::new(self.jump)
    }

    fn melee(&self) -> Box<dyn Display> {
        Box::new(self.melee)
    }

    fn grenade(&self) -> Box<dyn Display> {
        Box::new(self.grenade)
    }
}

#[expect(dead_code, reason = "reserved for future loadout builds")]
#[derive(Clone, Copy)]
pub(crate) enum Super {
    GlacialQuake,
}

impl Display for Super {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        let s = match self {
            Self::GlacialQuake => "Glacial Quake",
        };

        write!(f, "{s}")
    }
}

#[expect(dead_code, reason = "reserved for future loadout builds")]
#[derive(Clone, Copy)]
pub(crate) enum Melee {
    ShiverStrike,
}

impl Display for Melee {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        let s = match self {
            Self::ShiverStrike => "shiver_strike",
        };

        write!(f, "{s}")
    }
}

#[expect(dead_code, reason = "reserved for future loadout builds")]
#[derive(Clone, Copy)]
pub(crate) enum Aspect {
    Cryoclasm([StasisFragment; 3]),
    TectonicHarvest([StasisFragment; 2]),
    HowlOfTheStorm([StasisFragment; 2]),
    DiamondLance([StasisFragment; 3]),
}

impl AspectTrait for Aspect {
    fn fragments(&self) -> [Option<Box<dyn Display>>; 3] {
        match *self {
            Self::TectonicHarvest(fragments) | Self::HowlOfTheStorm(fragments) => [
                Some(box_display(fragments[0])),
                Some(box_display(fragments[1])),
                None,
            ],
            Self::Cryoclasm(fragments) | Self::DiamondLance(fragments) => [
                Some(box_display(fragments[0])),
                Some(box_display(fragments[1])),
                Some(box_display(fragments[2])),
            ],
        }
    }
}

impl Display for Aspect {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        let s = match self {
            Self::Cryoclasm(_) => "cryoclasm",
            Self::TectonicHarvest(_) => "tectonic_harvest",
            Self::HowlOfTheStorm(_) => "howl_of_the_storm",
            Self::DiamondLance(_) => "diamond_lance",
        };

        write!(f, "{s}")
    }
}
