use std::{fs::File, io::BufWriter, path::Path};

use anyhow::{bail, Result};

use crate::config::Config;

use super::godot_file::{Color, Field, GodotFile, GodotWriter, Tag, TagAssign, Value, Vector2i};

#[derive(Debug)]
pub struct TileSetResource {
    uid: String,
    pub texture_resource: TextureResource,
    pub tile_set_atlas_source: TileSetAtlasSource,
}

impl TileSetResource {
    pub(crate) fn init_from_file(file: GodotFile) -> Result<Self> {
        if file.header.name != "gd_resource" {
            bail!("expected a resource file, but found '{}'", file.header.name);
        };

        let Some(Field {
            value: Value::String(uid),
            ..
        }) = file
            .header
            .fields
            .into_iter()
            .find(|f| f.identifier == "uid")
        else {
            bail!("expected a uid string on 'gd_resource'");
        };

        let mut texture_resource = None;
        let mut tile_set_atlas_source = None;

        for tag in file.tags {
            match &*tag.name {
                "ext_resource" => {
                    if texture_resource.is_none() {
                        texture_resource = Some(TextureResource::init_from_tag(tag)?)
                    } else {
                        bail!("expected only one 'ext_resource'");
                    }
                }
                "sub_resource" => {
                    if tile_set_atlas_source.is_none() {
                        tile_set_atlas_source = Some(TileSetAtlasSource::init_from_tag(tag)?)
                    } else {
                        bail!("expected only one 'sub_resource'");
                    }
                }
                "resource" => {}
                other => bail!("unexpected tag '{other}'"),
            }
        }

        let Some(texture_resource) = texture_resource else {
            bail!("missing external 'Texture2D' resource");
        };

        let Some(tile_set_atlas_source) = tile_set_atlas_source else {
            bail!("missing 'TileSetAtlasSource' resource");
        };

        Ok(TileSetResource {
            uid,
            texture_resource,
            tile_set_atlas_source,
        })
    }

    pub(crate) fn print_to_file(&self, path: impl AsRef<Path>, config: &Config) -> Result<()> {
        let header = Tag {
            name: "gd_resource".into(),
            fields: vec![
                Field {
                    identifier: "type".into(),
                    value: Value::String("TileSet".into()),
                },
                Field {
                    identifier: "load_steps".into(),
                    value: Value::Integer(3), // self + resources
                },
                Field {
                    identifier: "format".into(),
                    value: Value::Integer(3),
                },
                Field {
                    identifier: "uid".into(),
                    value: Value::String(self.uid.clone()),
                },
            ],
            assigns: Vec::new(),
        };

        let image_tag = Tag {
            name: "ext_resource".into(),
            fields: vec![
                Field {
                    identifier: "type".into(),
                    value: Value::String("Texture2D".into()),
                },
                Field {
                    identifier: "uid".into(),
                    value: Value::String(self.texture_resource.uid.clone()),
                },
                Field {
                    identifier: "path".into(),
                    value: Value::String(self.texture_resource.path.clone()),
                },
                Field {
                    identifier: "id".into(),
                    value: Value::String(self.texture_resource.id.clone()),
                },
            ],
            assigns: Vec::new(),
        };

        let mut atlas_tag = Tag {
            name: "sub_resource".into(),
            fields: vec![
                Field {
                    identifier: "type".into(),
                    value: Value::String("TileSetAtlasSource".into()),
                },
                Field {
                    identifier: "id".into(),
                    value: Value::String(self.tile_set_atlas_source.id.clone()),
                },
            ],
            assigns: vec![
                TagAssign {
                    assign: "texture".into(),
                    value: Value::ExtResource(self.tile_set_atlas_source.texture.clone()),
                },
                TagAssign {
                    assign: "texture_region_size".into(),
                    value: Value::Vector2i(self.tile_set_atlas_source.texture_region_size),
                },
            ],
        };

        for tile in &self.tile_set_atlas_source.tiles {
            tile.append_assigns(&mut atlas_tag.assigns);
        }

        let mut resource_tag = Tag {
            name: "resource".into(),
            fields: Vec::new(),
            assigns: vec![
                TagAssign {
                    assign: "tile_shape".into(),
                    value: Value::Integer(3), // Hexagon
                },
                TagAssign {
                    assign: "tile_offset_axis".into(),
                    value: Value::Integer(1),
                },
                TagAssign {
                    assign: "tile_size".into(),
                    value: Value::Vector2i(self.tile_set_atlas_source.texture_region_size),
                },
            ],
        };

        for (set_index, terrain_set) in config.terrain_sets.iter().enumerate() {
            resource_tag.assigns.push(TagAssign {
                assign: format!("terrain_set_{set_index}/mode"),
                value: Value::Integer(2),
            });

            for (terrain_index, terrain) in terrain_set.terrains.iter().enumerate() {
                resource_tag.assigns.push(TagAssign {
                    assign: format!("terrain_set_{set_index}/terrain_{terrain_index}/name"),
                    value: Value::String(terrain.name.clone()),
                });

                resource_tag.assigns.push(TagAssign {
                    assign: format!("terrain_set_{set_index}/terrain_{terrain_index}/color"),
                    value: Value::Color(Color::Rgba(0.0, 0.0, 0.0, 1.0)),
                });
            }
        }

        resource_tag.assigns.push(TagAssign {
            assign: "sources/0".into(),
            value: Value::SubResource(self.tile_set_atlas_source.id.clone()),
        });

        let file = File::create(path)?;
        let mut writer = GodotWriter::begin(BufWriter::new(file), &header)?;
        writer.write_tag(&image_tag)?;
        writer.write_tag(&atlas_tag)?;
        writer.write_tag(&resource_tag)?;

        Ok(())
    }
}

#[derive(Debug)]
pub(crate) struct TextureResource {
    pub uid: String,
    pub path: String,
    pub id: String,
}

impl TextureResource {
    fn init_from_tag(tag: Tag) -> Result<Self> {
        let mut found_type = false;

        let mut resource = Self {
            uid: String::new(),
            path: String::new(),
            id: String::new(),
        };

        for field in tag.fields {
            match &*field.identifier {
                "type" => {
                    let Value::String(ty) = field.value else {
                        bail!("expected 'type' to be a string");
                    };

                    if ty != "Texture2D" {
                        bail!("expected texture resource type to be 'Texture2D'");
                    }

                    found_type = true;
                }
                "uid" => {
                    let Value::String(uid) = field.value else {
                        bail!("expected 'uid' to be a string");
                    };
                    resource.uid = uid;
                }
                "path" => {
                    let Value::String(path) = field.value else {
                        bail!("expected 'path' to be a string");
                    };
                    resource.path = path;
                }
                "id" => {
                    let Value::String(id) = field.value else {
                        bail!("expected 'id' to be a string");
                    };
                    resource.id = id;
                }
                other => bail!("unexpected 'ext_resource' field '{other}'"),
            }
        }

        if !found_type {
            bail!("expected texture resource type to be 'Texture2D'");
        }

        if resource.uid.is_empty() {
            bail!("missing texture resource 'uid'");
        }

        if resource.path.is_empty() {
            bail!("missing texture resource 'path'");
        }

        if resource.id.is_empty() {
            bail!("missing texture resource 'id'");
        }

        Ok(resource)
    }
}

#[derive(Debug)]
pub(crate) struct TileSetAtlasSource {
    id: String,
    texture: String,
    pub texture_region_size: Vector2i,
    pub tiles: Vec<Tile>,
}

impl TileSetAtlasSource {
    fn init_from_tag(tag: Tag) -> Result<Self> {
        let mut found_type = false;
        let mut id = String::new();
        let mut texture = String::new();

        for field in tag.fields {
            match &*field.identifier {
                "type" => {
                    let Value::String(ty) = field.value else {
                        bail!("expected 'type' to be a string");
                    };

                    if ty != "TileSetAtlasSource" {
                        bail!("expected tile atlas source type to be 'TileSetAtlasSource'");
                    }

                    found_type = true;
                }
                "id" => {
                    let Value::String(value) = field.value else {
                        bail!("expected 'id' to be a string");
                    };
                    id = value;
                }
                other => bail!("unexpected 'sub_resource' field '{other}'"),
            }
        }

        for assign in tag.assigns {
            match &*assign.assign {
                "texture" => {
                    let Value::ExtResource(value) = assign.value else {
                        bail!("expected 'texture' to be an 'ExtResource'");
                    };

                    texture = value;
                }
                _ => {}
            }
        }

        if !found_type {
            bail!("expected tile atlas source type to be 'TileSetAtlasSource'");
        }

        if id.is_empty() {
            bail!("missing tile atlas source 'id'");
        }

        if texture.is_empty() {
            bail!("missing tile atlas source 'texture'");
        }

        Ok(Self {
            id,
            texture,
            texture_region_size: Vector2i { x: 0, y: 0 },
            tiles: Vec::new(),
        })
    }
}

#[derive(Debug)]
pub(crate) struct Tile {
    pub position: Vector2i,
    pub terrain_set: Option<u32>,
    pub terrain: Option<u32>,
    pub terrains_peering_bit: PeeringBit,
}

impl Tile {
    fn append_assigns(&self, assigns: &mut Vec<TagAssign>) {
        let path = format!("{}:{}/0", self.position.x, self.position.y);

        assigns.push(TagAssign {
            assign: path.clone(),
            value: Value::Integer(0),
        });

        if let Some(terrain_set) = self.terrain_set {
            assigns.push(TagAssign {
                assign: format!("{path}/terrain_set"),
                value: Value::Integer(terrain_set as i64),
            });
        }

        if let Some(terrain) = self.terrain {
            assigns.push(TagAssign {
                assign: format!("{path}/terrain"),
                value: Value::Integer(terrain as i64),
            });
        }

        if let Some(bottom_right_side) = self.terrains_peering_bit.bottom_right_side {
            assigns.push(TagAssign {
                assign: format!("{path}/terrains_peering_bit/bottom_right_side"),
                value: Value::Integer(bottom_right_side as i64),
            });
        }

        if let Some(bottom_side) = self.terrains_peering_bit.bottom_side {
            assigns.push(TagAssign {
                assign: format!("{path}/terrains_peering_bit/bottom_side"),
                value: Value::Integer(bottom_side as i64),
            });
        }

        if let Some(bottom_left_side) = self.terrains_peering_bit.bottom_left_side {
            assigns.push(TagAssign {
                assign: format!("{path}/terrains_peering_bit/bottom_left_side"),
                value: Value::Integer(bottom_left_side as i64),
            });
        }

        if let Some(top_left_side) = self.terrains_peering_bit.top_left_side {
            assigns.push(TagAssign {
                assign: format!("{path}/terrains_peering_bit/top_left_side"),
                value: Value::Integer(top_left_side as i64),
            });
        }

        if let Some(top_side) = self.terrains_peering_bit.top_side {
            assigns.push(TagAssign {
                assign: format!("{path}/terrains_peering_bit/top_side"),
                value: Value::Integer(top_side as i64),
            });
        }

        if let Some(top_right_side) = self.terrains_peering_bit.top_right_side {
            assigns.push(TagAssign {
                assign: format!("{path}/terrains_peering_bit/top_right_side"),
                value: Value::Integer(top_right_side as i64),
            });
        }
    }
}

#[derive(Debug, Default)]
pub(crate) struct PeeringBit {
    pub bottom_right_side: Option<u32>,
    pub bottom_side: Option<u32>,
    pub bottom_left_side: Option<u32>,
    pub top_left_side: Option<u32>,
    pub top_side: Option<u32>,
    pub top_right_side: Option<u32>,
}
