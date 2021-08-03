use fount::FontData;
use swash::{CacheKey, FontRef, Synthesis};

#[derive(Clone)]
pub struct Font {
    pub data: FontData,
    pub offset: u32,
    pub key: CacheKey,
    pub synthesis: Synthesis,
}

impl Font {
    pub fn as_ref(&self) -> FontRef {
        FontRef {
            data: &self.data,
            offset: self.offset,
            key: self.key,
        }
    }
}

impl PartialEq for Font {
    fn eq(&self, other: &Self) -> bool {
        self.key == other.key && self.synthesis == other.synthesis
    }
}