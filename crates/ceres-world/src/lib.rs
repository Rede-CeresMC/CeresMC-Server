// println!/eprintln! of this crate go through the shared console (colours, log file)
#[allow(unused_macros)]
macro_rules! println {
    () => { ceres_codec::console::line("", false) };
    ($($t:tt)*) => { ceres_codec::console::line(&format!($($t)*), false) };
}
#[allow(unused_macros)]
macro_rules! eprintln {
    () => { ceres_codec::console::line("", true) };
    ($($t:tt)*) => { ceres_codec::console::line(&format!($($t)*), true) };
}

pub mod data;
pub mod world;

pub use data::{BreakInfo, Tool, ToolInfo, BlockStateInfo, CreativeEntry, GameData, Ingredient, ItemInfo, Recipe, extra_data};
pub use world::{MAX_Y, MIN_Y, Weather, World, WorldManager};
