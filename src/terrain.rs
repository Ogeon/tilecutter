use core::str;
use std::{fs::File, io::BufReader, os::unix::ffi::OsStrExt, path::Path};

use anyhow::{anyhow, bail, Context, Result};
use image::{GenericImage, GenericImageView, Rgba, RgbaImage};
use itertools::Itertools;

use crate::{
    config::{Config, TerrainSetConfig},
    godot::resource::PeeringBit,
};

const MASK_COLORS: [Rgba<u8>; 6] = [
    Rgba([255, 0, 0, 255]),
    Rgba([0, 255, 0, 255]),
    Rgba([0, 0, 255, 255]),
    Rgba([0, 255, 255, 255]),
    Rgba([255, 0, 255, 255]),
    Rgba([255, 255, 0, 255]),
];

pub(crate) fn load_terrain_tiles(config_path: &Path, config: &Config) -> Result<Vec<TerrainTile>> {
    if config.terrain_sets.is_empty() {
        return Ok(Vec::new());
    }

    let directory_path = config_path.join("terrains");

    let mask_image_path = directory_path.join("mask.png");
    let mask_image_file = File::open(&mask_image_path)
        .with_context(|| format!("could not open mask image {mask_image_path:?}"))?;
    let mask_image_file = BufReader::new(mask_image_file);
    let mask_image = image::load(mask_image_file, image::ImageFormat::Png)
        .with_context(|| format!("could not load mask image {mask_image_path:?}"))?
        .into_rgba8();

    let images = load_images(&directory_path, config)?;
    let combinations = find_combinations(config, &images);

    let mut tiles = Vec::new();
    for combination in combinations {
        tiles.extend(generate_combinations(&combination, &images, &mask_image));
    }

    Ok(tiles)
}

fn find_combinations(config: &Config, images: &[TerrainImage]) -> Vec<Vec<TerrainId>> {
    let mut possible_combinations = Vec::new();

    for (set_index, set) in config.terrain_sets.iter().enumerate() {
        // A hexagon can have at most 6 different neighbors, meaning we only need to
        // consider combination of at most 7 terrains.
        for length in 1..8 {
            let combinations = set
                .terrains
                .iter()
                .enumerate()
                .map(|(terrain_index, _)| TerrainId {
                    terrain_set: set_index,
                    terrain: terrain_index,
                })
                .combinations(length);

            for combination in combinations {
                if has_images_for_combination(images, &combination) {
                    eprintln!("adding combination {combination:?}");
                    possible_combinations.push(combination);
                }
            }
        }
    }

    possible_combinations
}

fn load_images(directory_path: &Path, config: &Config) -> Result<Vec<TerrainImage>> {
    let mut terrain_images = Vec::new();

    let [tile_width, tile_height] = config.tile_set.tile_size;
    let expected_sizes = [
        [tile_width, tile_height * 4],
        [tile_width, tile_height * 4],
        [tile_width, tile_height * 2],
    ];

    for entry in std::fs::read_dir(&directory_path)
        .with_context(|| format!("could not open {directory_path:?}"))?
    {
        let entry =
            entry.with_context(|| format!("could not read content of {directory_path:?}"))?;

        let path = entry.path();

        let Some(extension) = path.extension() else {
            continue;
        };

        if extension != "png" {
            continue;
        }

        let Some(stem) = path.file_stem() else {
            continue;
        };

        if !stem.is_ascii() {
            continue;
        }

        let stem = str::from_utf8(stem.as_bytes()).expect("file name should be valid UTF-8");

        if stem == "mask" {
            continue;
        }

        let mut parts = stem.split('-');
        let terrain_names = parts.by_ref().take(3).map(str::trim).collect::<Vec<_>>();

        if parts.next().is_some() {
            bail!(
                "expected file name to have the format 'TerrainName.png', 'TerrainName-OtherName.png', or 'TerrainName-Other1Name-Other2Name.png' for {path:?}",
            );
        }

        let terrains = terrain_names
            .iter()
            .map(|name| {
                find_terrain(name, &config.terrain_sets)
                    .ok_or_else(|| anyhow!("'{name}' is not a known terrain"))
            })
            .collect::<Result<Vec<_>, _>>()?;

        let image_file = File::open(&path).with_context(|| format!("could not open {path:?}"))?;
        let image_file = BufReader::new(image_file);
        let image = image::load(image_file, image::ImageFormat::Png)
            .with_context(|| format!("could not load {path:?}"))?
            .into_rgba8();

        let center_terrain = terrains[0];

        if let Some(other) = terrains.get(1) {
            if other.terrain_set != center_terrain.terrain_set {
                eprintln!(
                    "'{}' and '{}' are not in the same terrain sets",
                    terrain_names[0], terrain_names[1]
                );
                continue;
            }
        }

        if let Some(other) = terrains.get(2) {
            if other.terrain_set != center_terrain.terrain_set {
                eprintln!(
                    "'{}' and '{}' are not in the same terrain sets",
                    terrain_names[0], terrain_names[2]
                );
                continue;
            }
        }

        let expected_size = expected_sizes[terrains.len() - 1];
        if [image.width(), image.height()] != expected_size {
            bail!(
                "expected an image of size {}x{}, but found  {}x{} in {path:?}",
                expected_size[0],
                expected_size[1],
                image.width(),
                image.height()
            );
        }

        terrain_images.push(TerrainImage {
            combination: terrains,
            image,
        })
    }

    Ok(terrain_images)
}

fn generate_combinations(
    terrains: &[TerrainId],
    images: &[TerrainImage],
    mask_image: &RgbaImage,
) -> Vec<TerrainTile> {
    let mut tiles = Vec::new();
    let center_terrain = terrains[0];
    let main_image = images
        .iter()
        .find(|image| image.combination == [center_terrain])
        .expect("an image for only the center terrain should exist");
    let none_image = main_image
        .image
        .view(0, 0, main_image.image.width(), mask_image.height());

    for sides in itertools::repeat_n(
        std::iter::once(None).chain(terrains.iter().copied().map(Some)),
        6,
    )
    .multi_cartesian_product()
    {
        let mut image = RgbaImage::new(mask_image.width(), mask_image.height());
        image
            .copy_from(&*none_image, 0, 0)
            .expect("combination image should fit a tile");

        for ((index, side), (_, next)) in sides.iter().copied().enumerate().circular_tuple_windows()
        {
            let mut combination = get_terrain_combination(center_terrain, side, next);
            let (combo_image, swapped) = find_image_for_combination(images, &mut combination)
                .expect("combination should have an image");

            let sub_image_index = match &*combination {
                &[_] => match (side.is_some(), next.is_some()) {
                    (true, true) => 3,
                    (true, false) => 1 + index as u32 % 2,
                    (false, true) => 2 - index as u32 % 2,
                    (false, false) => continue,
                },
                _ => unimplemented!(),
            };

            let source = combo_image.image.view(
                0,
                mask_image.height() * sub_image_index,
                combo_image.image.width(),
                mask_image.height(),
            );

            let mask_color = MASK_COLORS[index];
            for ((dst, (_, _, src)), mask) in image
                .pixels_mut()
                .zip(source.pixels())
                .zip(mask_image.pixels())
            {
                if *mask == mask_color {
                    *dst = src
                }
            }
        }

        tiles.push(TerrainTile {
            terrain: center_terrain,
            terrains_peering_bit: sides_to_peering_bit(&sides),
            image,
        });
    }

    tiles
}

fn find_image_for_combination<'a>(
    images: &'a [TerrainImage],
    combination: &mut [TerrainId],
) -> Option<(&'a TerrainImage, bool)> {
    let found_image = images.iter().find(|image| image.combination == combination);

    if found_image.is_none() && combination.len() == 3 {
        combination.swap(2, 2);

        images
            .iter()
            .find(|image| image.combination == combination)
            .map(|image| (image, true))
    } else {
        found_image.map(|image| (image, false))
    }
}

fn get_terrain_combination(
    center_terrain: TerrainId,
    side1: Option<TerrainId>,
    side2: Option<TerrainId>,
) -> Vec<TerrainId> {
    match (side1, side2) {
        (None, None) => vec![center_terrain],
        (None, Some(other)) | (Some(other), None) if other == center_terrain => {
            vec![center_terrain]
        }
        (Some(other1), Some(other2)) if other1 == center_terrain && other2 == center_terrain => {
            vec![center_terrain]
        }
        (None, Some(other)) | (Some(other), None) => vec![center_terrain, other],
        (Some(other1), Some(other2)) => vec![center_terrain, other1, other2],
    }
}

fn find_terrain(name: &str, terrain_sets: &[TerrainSetConfig]) -> Option<TerrainId> {
    terrain_sets
        .iter()
        .enumerate()
        .find_map(|(set_index, set)| {
            let terrain_index = set
                .terrains
                .iter()
                .enumerate()
                .find_map(|(index, terrain)| (terrain.name == name).then_some(index))?;

            Some(TerrainId {
                terrain_set: set_index,
                terrain: terrain_index,
            })
        })
}

fn has_images_for_combination(images: &[TerrainImage], combination: &[TerrainId]) -> bool {
    let mut matches_one_to_any = false;
    let mut matches_one_to_one = combination.len() < 2;
    let mut matches_one_to_two = combination.len() < 3;

    if let &[checked_terrain, ..] = combination {
        matches_one_to_any |= images.iter().any(|image| {
            if let &[terrain] = &*image.combination {
                terrain == checked_terrain
            } else {
                false
            }
        });
    }

    if combination.len() >= 2 {
        for transition in combination.iter().copied().permutations(2) {
            let &[checked_terrain, checked_other] = &*transition else {
                unreachable!()
            };

            matches_one_to_one |= images.iter().any(|image| {
                if let &[terrain, other] = &*image.combination {
                    terrain == checked_terrain && other == checked_other
                } else {
                    false
                }
            });
        }
    }

    if combination.len() >= 3 {
        for transition in combination.iter().copied().permutations(3) {
            let &[checked_terrain, checked_other1, checked_other2] = &*transition else {
                unreachable!()
            };

            matches_one_to_two |= images.iter().any(|image| {
                if let &[terrain, other1, other2] = &*image.combination {
                    terrain == checked_terrain
                        && other1 == checked_other1
                        && other2 == checked_other2
                } else {
                    false
                }
            });
        }
    }

    matches_one_to_any && matches_one_to_one && matches_one_to_two
}

fn sides_to_peering_bit(sides: &[Option<TerrainId>]) -> PeeringBit {
    assert_eq!(sides.len(), 6);

    PeeringBit {
        top_left_side: sides[0].map(|t| t.terrain as u32),
        top_side: sides[1].map(|t| t.terrain as u32),
        top_right_side: sides[2].map(|t| t.terrain as u32),
        bottom_right_side: sides[3].map(|t| t.terrain as u32),
        bottom_side: sides[4].map(|t| t.terrain as u32),
        bottom_left_side: sides[5].map(|t| t.terrain as u32),
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) struct TerrainId {
    pub terrain_set: usize,
    pub terrain: usize,
}

pub(crate) struct TerrainTile {
    pub terrain: TerrainId,
    pub terrains_peering_bit: PeeringBit,
    pub image: RgbaImage,
}

struct TerrainImage {
    combination: Vec<TerrainId>,
    image: RgbaImage,
}
