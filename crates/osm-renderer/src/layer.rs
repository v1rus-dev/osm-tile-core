use std::cmp::Ordering;

use crate::RenderError;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LayerId(String);

impl LayerId {
    pub fn new(id: impl Into<String>) -> Result<Self, RenderError> {
        let id = id.into();
        if id.trim().is_empty() {
            return Err(RenderError::InvalidLayerId);
        }

        Ok(Self(id))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<LayerId> for String {
    fn from(value: LayerId) -> Self {
        value.0
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct LayerCommon {
    pub id: LayerId,
    pub visible: bool,
    pub opacity: f32,
    pub z_index: i32,
}

impl LayerCommon {
    pub fn new(id: LayerId, z_index: i32) -> Self {
        Self {
            id,
            visible: true,
            opacity: 1.0,
            z_index,
        }
    }

    pub fn validate(self) -> Result<Self, RenderError> {
        validate_opacity(self.opacity)?;
        Ok(self)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TileLayer {
    pub common: LayerCommon,
    pub url_template: String,
    pub min_zoom: u32,
    pub max_zoom: u32,
}

impl TileLayer {
    pub fn new(id: LayerId, url_template: impl Into<String>, z_index: i32) -> Self {
        Self {
            common: LayerCommon::new(id, z_index),
            url_template: url_template.into(),
            min_zoom: 0,
            max_zoom: 30,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct MarkerLayer {
    pub common: LayerCommon,
}

#[derive(Debug, Clone, PartialEq)]
pub struct VectorLayer {
    pub common: LayerCommon,
}

#[derive(Debug, Clone, PartialEq)]
pub enum MapLayer {
    Tile(TileLayer),
    Marker(MarkerLayer),
    Vector(VectorLayer),
}

impl MapLayer {
    pub fn common(&self) -> &LayerCommon {
        match self {
            Self::Tile(layer) => &layer.common,
            Self::Marker(layer) => &layer.common,
            Self::Vector(layer) => &layer.common,
        }
    }

    pub fn common_mut(&mut self) -> &mut LayerCommon {
        match self {
            Self::Tile(layer) => &mut layer.common,
            Self::Marker(layer) => &mut layer.common,
            Self::Vector(layer) => &mut layer.common,
        }
    }

    pub fn validate(self) -> Result<Self, RenderError> {
        match self {
            Self::Tile(mut layer) => {
                layer.common = layer.common.validate()?;
                if layer.min_zoom > layer.max_zoom || layer.max_zoom > 30 {
                    return Err(RenderError::InvalidLayerZoomRange {
                        min_zoom: layer.min_zoom,
                        max_zoom: layer.max_zoom,
                    });
                }
                Ok(Self::Tile(layer))
            }
            Self::Marker(mut layer) => {
                layer.common = layer.common.validate()?;
                Ok(Self::Marker(layer))
            }
            Self::Vector(mut layer) => {
                layer.common = layer.common.validate()?;
                Ok(Self::Vector(layer))
            }
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct LayerStack {
    layers: Vec<MapLayer>,
}

impl LayerStack {
    pub fn new() -> Self {
        Self { layers: Vec::new() }
    }

    pub fn layers(&self) -> &[MapLayer] {
        &self.layers
    }

    pub fn add_or_replace(&mut self, layer: MapLayer) -> Result<(), RenderError> {
        let layer = layer.validate()?;
        let id = layer.common().id.clone();

        if let Some(existing) = self
            .layers
            .iter_mut()
            .find(|existing| existing.common().id == id)
        {
            *existing = layer;
        } else {
            self.layers.push(layer);
        }

        self.sort_layers();
        Ok(())
    }

    pub fn remove(&mut self, id: &LayerId) -> Option<MapLayer> {
        let index = self
            .layers
            .iter()
            .position(|layer| &layer.common().id == id)?;
        Some(self.layers.remove(index))
    }

    pub fn set_visible(&mut self, id: &LayerId, visible: bool) -> Result<(), RenderError> {
        let layer = self.layer_mut(id)?;
        layer.common_mut().visible = visible;
        Ok(())
    }

    pub fn set_opacity(&mut self, id: &LayerId, opacity: f32) -> Result<(), RenderError> {
        validate_opacity(opacity)?;
        let layer = self.layer_mut(id)?;
        layer.common_mut().opacity = opacity;
        Ok(())
    }

    fn layer_mut(&mut self, id: &LayerId) -> Result<&mut MapLayer, RenderError> {
        self.layers
            .iter_mut()
            .find(|layer| &layer.common().id == id)
            .ok_or_else(|| RenderError::MissingLayer {
                id: id.as_str().to_owned(),
            })
    }

    fn sort_layers(&mut self) {
        self.layers.sort_by(|left, right| {
            left.common()
                .z_index
                .cmp(&right.common().z_index)
                .then_with(|| match (left, right) {
                    (MapLayer::Tile(_), MapLayer::Tile(_)) => Ordering::Equal,
                    (MapLayer::Tile(_), _) => Ordering::Less,
                    (_, MapLayer::Tile(_)) => Ordering::Greater,
                    (MapLayer::Vector(_), MapLayer::Marker(_)) => Ordering::Less,
                    (MapLayer::Marker(_), MapLayer::Vector(_)) => Ordering::Greater,
                    _ => Ordering::Equal,
                })
                .then_with(|| left.common().id.as_str().cmp(right.common().id.as_str()))
        });
    }
}

fn validate_opacity(opacity: f32) -> Result<(), RenderError> {
    if !opacity.is_finite() || !(0.0..=1.0).contains(&opacity) {
        return Err(RenderError::InvalidLayerOpacity { opacity });
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(value: &str) -> LayerId {
        LayerId::new(value).unwrap()
    }

    #[test]
    fn layer_stack_sorts_by_z_index_type_and_id() {
        let mut stack = LayerStack::new();
        stack
            .add_or_replace(MapLayer::Marker(MarkerLayer {
                common: LayerCommon::new(id("markers"), 10),
            }))
            .unwrap();
        stack
            .add_or_replace(MapLayer::Tile(TileLayer::new(
                id("base"),
                "http://localhost/{z}/{x}/{y}.png",
                0,
            )))
            .unwrap();
        stack
            .add_or_replace(MapLayer::Vector(VectorLayer {
                common: LayerCommon::new(id("route"), 10),
            }))
            .unwrap();

        let ids = stack
            .layers()
            .iter()
            .map(|layer| layer.common().id.as_str())
            .collect::<Vec<_>>();

        assert_eq!(ids, vec!["base", "route", "markers"]);
    }

    #[test]
    fn layer_stack_replaces_layer_with_same_id() {
        let mut stack = LayerStack::new();
        stack
            .add_or_replace(MapLayer::Tile(TileLayer::new(id("base"), "a", 0)))
            .unwrap();
        stack
            .add_or_replace(MapLayer::Tile(TileLayer::new(id("base"), "b", 5)))
            .unwrap();

        assert_eq!(stack.layers().len(), 1);
        assert_eq!(stack.layers()[0].common().z_index, 5);
    }

    #[test]
    fn layer_stack_updates_visibility_and_opacity() {
        let mut stack = LayerStack::new();
        let layer_id = id("base");
        stack
            .add_or_replace(MapLayer::Tile(TileLayer::new(layer_id.clone(), "a", 0)))
            .unwrap();

        stack.set_visible(&layer_id, false).unwrap();
        stack.set_opacity(&layer_id, 0.5).unwrap();

        assert!(!stack.layers()[0].common().visible);
        assert_eq!(stack.layers()[0].common().opacity, 0.5);
    }

    #[test]
    fn rejects_invalid_layer_values() {
        let mut stack = LayerStack::new();
        assert!(matches!(
            LayerId::new(" "),
            Err(RenderError::InvalidLayerId)
        ));
        assert!(matches!(
            stack.add_or_replace(MapLayer::Tile(TileLayer {
                common: LayerCommon {
                    id: id("bad"),
                    visible: true,
                    opacity: 1.5,
                    z_index: 0,
                },
                url_template: "a".to_owned(),
                min_zoom: 0,
                max_zoom: 30,
            })),
            Err(RenderError::InvalidLayerOpacity { .. })
        ));
    }
}
