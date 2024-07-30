use std::{
    fs::File,
    io::Read,
    path::{Path, PathBuf},
};

use anyhow::{bail, Context, Result};
use clap::Parser;
use config::{Config, GodotConfig};
use godot::{resource::TileSetResource, Vector2i};
use image::{GenericImage, RgbaImage};
use terrain::{load_terrain_tiles, TerrainTile};
use tile::{load_tiles, Tile};

mod config;
mod godot;
mod terrain;
mod tile;

#[derive(clap::Parser)]
#[command(version, about, long_about = None)]
struct Args {
    file: String,
    #[arg(long, short)]
    dry_run: bool,
}

fn main() {
    let args = Args::parse();

    if let Err(error) = try_run(args) {
        eprintln!("could not export tile set: {error:?}");
    }
}

fn try_run(args: Args) -> Result<()> {
    // Load and check config.
    let config = load_config(&args.file).context("could not read tile set config file")?;
    if !config.godot.tile_set_path.ends_with(".tres") {
        bail!("expected 'tile_set_path' to be on the format 'res://Path/To/resource.tres'");
    }

    // Find paths.
    let config_directory_path = AsRef::<Path>::as_ref(&args.file)
        .parent()
        .expect("could not make a parent path for the config path")
        .to_owned();
    let godot_project_path = config_directory_path.join(&config.godot.project_path);
    let resource_path = godot_path_to_absolute(&godot_project_path, &config.godot.tile_set_path)?;

    // Load current Godot resource file.
    let mut resource =
        load_godot_resource(&resource_path).context("could not load Godot tile set file")?;

    // Load and generate tile sheet.
    let tiles = load_tiles(&config_directory_path, &config)?;
    let terrain_tiles = load_terrain_tiles(&config_directory_path, &config)?;
    let (image, layout) = write_tile_set_image(&tiles, terrain_tiles, &config);

    // Update resource data.
    resource.tile_set_atlas_source.texture_region_size = Vector2i::from(config.tile_set.tile_size);
    resource.tile_set_atlas_source.tiles = layout;
    let texture_path = if resource.texture_resource.path.is_empty() {
        bail!("expected a tile set texture to have been added in the resource file via Godot");
    } else {
        godot_path_to_absolute(&godot_project_path, &resource.texture_resource.path)?
    };

    // Write resource files.
    resource.print_to_file(resource_path, &config)?;
    image.save_with_format(texture_path, image::ImageFormat::Png)?;

    Ok(())
}

fn load_config(path: &str) -> Result<Config> {
    let mut config_content = String::new();
    File::open(path)?.read_to_string(&mut config_content)?;
    toml::from_str::<Config>(&config_content).context("could not parse tile set config")
}

fn load_godot_resource(resource_path: &Path) -> Result<TileSetResource> {
    let godot_file = godot::parse_file(&resource_path)
        .with_context(|| format!("could not parse {resource_path:?} as a '*.tres' file"))?;

    godot::resource::TileSetResource::init_from_file(godot_file)
        .context("unexpected '*.tres' file content")
}

fn godot_path_to_absolute(project_path: &Path, godot_path: &str) -> Result<PathBuf> {
    if !godot_path.starts_with("res://") {
        bail!("expected a Godot path on the format 'res://Path/To/resourc'");
    }

    Ok(project_path.join(godot_path.trim_start_matches("res://")))
}

fn write_tile_set_image(
    tiles: &[Tile],
    terrain_tiles: Vec<TerrainTile>,
    config: &Config,
) -> (RgbaImage, Vec<godot::resource::Tile>) {
    let [tile_width, tile_height] = config.tile_set.tile_size;
    let total_tiles = tiles.len() as u32 + terrain_tiles.len() as u32;
    let mut layout = Vec::new();
    let mut image_size = 0;

    for tile in tiles {
        let [x, y] = tile.config.position;

        let req_width = (x + 1) * tile_width;
        let req_height = (y + 1) * tile_height;
        let req_size = req_width.max(req_height);

        image_size = image_size.max(req_size);
    }

    while (image_size / tile_width) * (image_size / tile_height) < total_tiles {
        image_size += tile_width.max(tile_height);
    }

    let mut image = RgbaImage::new(image_size, image_size);

    for tile in tiles {
        let [x, y] = tile.config.position;

        image
            .copy_from(&tile.image, x * tile_width, y * tile_height)
            .expect("there should be enough room in the image for the tiles");

        layout.push(godot::resource::Tile {
            position: Vector2i::from([x, y]),
            terrain_set: None,
            terrain: None,
            terrains_peering_bit: Default::default(),
        })
    }

    let coordinates = (0..(image_size / tile_height))
        .flat_map(|y| (0..(image_size / tile_width)).map(move |x| (x, y)))
        .filter(|&(x, y)| !tiles.iter().any(|tile| tile.config.position == [x, y]));

    for ((x, y), tile) in coordinates.zip(terrain_tiles) {
        image
            .copy_from(&tile.image, x * tile_width, y * tile_height)
            .expect("there should be enough room in the image for the terrain tiles");

        layout.push(godot::resource::Tile {
            position: Vector2i::from([x, y]),
            terrain_set: Some(tile.terrain.terrain_set as u32),
            terrain: Some(tile.terrain.terrain as u32),
            terrains_peering_bit: tile.terrains_peering_bit,
        })
    }

    (image, layout)
}
