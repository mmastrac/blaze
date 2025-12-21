pub struct Color {
    pub background: (u8, u8, u8),
    pub foreground: (u8, u8, u8),
    pub bold: (u8, u8, u8),
}

pub const DEFAULT_COLOR: Color = Color {
    background: (10, 10, 10),
    foreground: (207, 159, 64),
    bold: (249, 218, 76),
};

pub const GRAYSCALE_COLOR: Color = Color {
    background: (10, 10, 10),
    foreground: (80, 80, 80),
    bold: (224, 224, 224),
};
