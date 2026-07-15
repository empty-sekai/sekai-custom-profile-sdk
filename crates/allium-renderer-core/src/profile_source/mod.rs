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
