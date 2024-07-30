use serde::Deserialize;

#[derive(Deserialize, Debug)]
pub(crate) struct Config {
    pub tile_set: TileSetConfig,
    pub godot: GodotConfig,
    #[serde(default)]
    pub tiles: Vec<TileConfig>,
    #[serde(default)]
    pub terrain_sets: Vec<TerrainSetConfig>,
}

#[derive(Deserialize, Debug)]
pub(crate) struct TileSetConfig {
    pub tile_size: [u32; 2],
}

#[derive(Deserialize, Debug)]
pub(crate) struct GodotConfig {
    pub project_path: String,
    pub tile_set_path: String,
}

#[derive(Deserialize, Debug)]
pub(crate) struct TileConfig {
    pub name: String,
    pub position: [u32; 2],
}

#[derive(Deserialize, Debug)]
pub(crate) struct TerrainSetConfig {
    #[serde(default)]
    pub terrains: Vec<TerrainConfig>,
}

#[derive(Deserialize, Debug)]
pub(crate) struct TerrainConfig {
    pub name: String,
}
