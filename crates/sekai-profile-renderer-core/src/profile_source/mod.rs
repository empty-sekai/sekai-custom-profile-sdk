//! Input model: the card and element shapes as they arrive from the game API.
//!
//! These types mirror the payload rather than the renderer's needs. Lowering
//! them into [`crate::profile_scene`] snapshots is what makes them drawable.

mod card;
mod elements;

pub use card::*;
pub use elements::*;

impl CustomProfileCard {
    pub fn element_count(&self) -> usize {
        self.texts.len()
            + self.shapes.len()
            + self.card_members.len()
            + self.stamps.len()
            + self.others.len()
            + self.bonds_honors.len()
            + self.honors.len()
            + self.collections.len()
            + self.generals.len()
            + self.stand_members.len()
            + self.general_backgrounds.len()
            + self.story_backgrounds.len()
    }
}
