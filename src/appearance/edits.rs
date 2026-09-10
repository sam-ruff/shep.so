//! Fixed-size local save intent, independent of the shared-profile wire format.
use super::{Palettes, Rgb, Role};

const COUNT: usize = Role::ALL.len();

#[derive(Clone, Debug, Default)]
pub struct Edits {
    // Keep the bounded command queue compact; only saves allocate this payload.
    values: Box<[[Option<Rgb>; COUNT]; 2]>,
}
impl Edits {
    pub fn all(palettes: Palettes) -> Self {
        let mut edits = Self::default();
        for dark in [false, true] {
            for &role in Role::ALL {
                edits.values[usize::from(dark)][role as usize] = Some(palettes.get(dark).get(role));
            }
        }
        edits
    }
    pub fn apply(&self, palettes: &mut Palettes) {
        for dark in [false, true] {
            for &role in Role::ALL {
                if let Some(color) = self.values[usize::from(dark)][role as usize] {
                    palettes.get_mut(dark).set(role, color);
                }
            }
        }
    }
}

pub(crate) struct Tracker {
    observed: Palettes,
    generations: [[u64; COUNT]; 2],
}
impl Tracker {
    pub fn new(palettes: Palettes) -> Self {
        Self {
            observed: palettes,
            generations: [[0; COUNT]; 2],
        }
    }
    pub fn capture(&mut self, palettes: Palettes, generation: u64) {
        for dark in [false, true] {
            for &role in Role::ALL {
                if self.observed.get(dark).get(role) != palettes.get(dark).get(role) {
                    self.generations[usize::from(dark)][role as usize] = generation;
                }
            }
        }
        self.observed = palettes;
    }
    pub fn edits(&self, palettes: Palettes, acknowledged: u64) -> Edits {
        let mut edits = Edits::default();
        for dark in [false, true] {
            for &role in Role::ALL {
                if self.generations[usize::from(dark)][role as usize] > acknowledged {
                    edits.values[usize::from(dark)][role as usize] =
                        Some(palettes.get(dark).get(role));
                }
            }
        }
        edits
    }
    pub fn observe(&mut self, saved: Palettes, live: &mut Palettes, acknowledged: u64) {
        for dark in [false, true] {
            for &role in Role::ALL {
                if self.generations[usize::from(dark)][role as usize] <= acknowledged {
                    live.get_mut(dark).set(role, saved.get(dark).get(role));
                }
            }
        }
        self.observed = *live;
    }
}
