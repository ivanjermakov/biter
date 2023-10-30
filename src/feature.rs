pub enum Feature {
    Dht,
    Extension,
}

impl Feature {
    pub fn new_with(features: &[Feature]) -> Vec<u8> {
        let mut reserved = vec![0u8; 8];
        for f in features {
            f.enable(&mut reserved);
        }
        reserved
    }

    pub fn bit(&self) -> (usize, u8) {
        match &self {
            Feature::Dht => (7, 0x01),
            Feature::Extension => (5, 0x10),
        }
    }

    pub fn enable(&self, features: &mut [u8]) {
        let (i, b) = self.bit();
        features[i] |= b;
    }

    pub fn enabled(&self, features: &[u8]) -> bool {
        let (i, b) = self.bit();
        features[i] & b != 0
    }
}
