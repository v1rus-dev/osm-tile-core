use crate::{LayerStack, MapCamera, RenderError, RenderViewport, VisibleTile, visible_tiles};

#[derive(Debug, Clone)]
pub struct RenderState {
    camera: MapCamera,
    viewport: Option<RenderViewport>,
    layers: LayerStack,
}

impl Default for RenderState {
    fn default() -> Self {
        Self::new()
    }
}

impl RenderState {
    pub fn new() -> Self {
        Self {
            camera: MapCamera::default(),
            viewport: None,
            layers: LayerStack::new(),
        }
    }

    pub fn camera(&self) -> MapCamera {
        self.camera
    }

    pub fn set_camera(&mut self, camera: MapCamera) -> Result<(), RenderError> {
        self.camera = camera.validate()?;
        Ok(())
    }

    pub fn viewport(&self) -> Option<RenderViewport> {
        self.viewport
    }

    pub fn set_viewport(&mut self, viewport: RenderViewport) -> Result<(), RenderError> {
        self.viewport = Some(viewport.validate()?);
        Ok(())
    }

    pub fn layers(&self) -> &LayerStack {
        &self.layers
    }

    pub fn layers_mut(&mut self) -> &mut LayerStack {
        &mut self.layers
    }

    pub fn visible_tiles(&self) -> Result<Vec<VisibleTile>, RenderError> {
        let viewport = self.viewport.ok_or(RenderError::MissingRenderViewport)?;
        visible_tiles(self.camera, viewport)
    }
}

#[cfg(test)]
mod tests {
    use crate::{LayerId, MapLayer, TileLayer};

    use super::*;

    #[test]
    fn camera_pan_changes_visible_tiles() {
        let mut state = RenderState::new();
        state
            .set_viewport(RenderViewport::new(256, 256, 1.0).unwrap())
            .unwrap();
        state
            .set_camera(MapCamera::new(0.0, 0.0, 2.0).unwrap())
            .unwrap();
        let first = state.visible_tiles().unwrap();

        state
            .set_camera(MapCamera::new(0.0, 90.0, 2.0).unwrap())
            .unwrap();
        let second = state.visible_tiles().unwrap();

        assert_ne!(first, second);
    }

    #[test]
    fn requires_viewport_before_visible_tiles() {
        let state = RenderState::new();

        assert!(matches!(
            state.visible_tiles(),
            Err(RenderError::MissingRenderViewport)
        ));
    }

    #[test]
    fn stores_layer_stack() {
        let mut state = RenderState::new();
        state
            .layers_mut()
            .add_or_replace(MapLayer::Tile(TileLayer::new(
                LayerId::new("base").unwrap(),
                "http://localhost/{z}/{x}/{y}.png",
                0,
            )))
            .unwrap();

        assert_eq!(state.layers().layers().len(), 1);
    }
}
