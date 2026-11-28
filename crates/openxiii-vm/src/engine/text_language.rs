use crate::Vm;

impl Vm {
    /// Language of the game's texts (a localization extension: "int", "frt"…).
    pub fn text_language(&self) -> &str {
        self.config.as_ref().map_or("int", |c| c.language.as_str())
    }
}
