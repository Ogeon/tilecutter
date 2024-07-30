use std::{fs::File, io::BufReader, path::Path};

use anyhow::{bail, Context, Result};
use image::RgbaImage;

use crate::config::{Config, TileConfig};

pub(crate) struct Tile<'a> {
    pub config: &'a TileConfig,
    pub image: RgbaImage,
}

pub(crate) fn load_tiles<'a>(config_path: &Path, config: &'a Config) -> Result<Vec<Tile<'a>>> {
    let directory_path = config_path.join("tiles");

    let mut tiles = vec![];

    for tile in &config.tiles {
        let path = directory_path.join(format!("{}.png", tile.name));
        let image_file = File::open(&path).with_context(|| format!("could not open {path:?}"))?;
        let image_file = BufReader::new(image_file);
        let image = image::load(image_file, image::ImageFormat::Png)
            .with_context(|| format!("could not load {path:?}"))?
            .into_rgba8();

        if [image.width(), image.height()] != config.tile_set.tile_size {
            bail!(
                "expected an image of size {}x{}, but found  {}x{} in {path:?}",
                config.tile_set.tile_size[0],
                config.tile_set.tile_size[1],
                image.width(),
                image.height()
            );
        }

        tiles.push(Tile {
            config: tile,
            image,
        })
    }

    Ok(tiles)
}
