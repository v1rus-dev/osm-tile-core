use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use tokio::fs;
use tokio::io::AsyncWriteExt;

use crate::{TileError, TileId};

#[derive(Debug, Clone)]
pub struct FileTileCache {
    root: PathBuf,
}

impl FileTileCache {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub async fn contains(&self, id: TileId) -> Result<bool, TileError> {
        let id = id.validate()?;

        match fs::metadata(self.tile_path(id)).await {
            Ok(metadata) => Ok(metadata.is_file()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(TileError::CacheIo(error)),
        }
    }

    pub async fn get(&self, id: TileId) -> Result<Option<Vec<u8>>, TileError> {
        let id = id.validate()?;

        match fs::read(self.tile_path(id)).await {
            Ok(bytes) => Ok(Some(bytes)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(TileError::CacheIo(error)),
        }
    }

    pub async fn put(&self, id: TileId, bytes: &[u8]) -> Result<(), TileError> {
        let id = id.validate()?;
        let path = self.tile_path(id);
        let parent = path.parent().ok_or(TileError::InvalidCachePath)?;

        fs::create_dir_all(parent)
            .await
            .map_err(TileError::CacheIo)?;

        let temporary_path = self.temporary_tile_path(id);
        let mut file = fs::File::create(&temporary_path)
            .await
            .map_err(TileError::CacheIo)?;

        file.write_all(bytes).await.map_err(TileError::CacheIo)?;
        file.flush().await.map_err(TileError::CacheIo)?;
        drop(file);

        fs::rename(&temporary_path, &path)
            .await
            .map_err(TileError::CacheIo)?;

        Ok(())
    }

    pub async fn remove(&self, id: TileId) -> Result<(), TileError> {
        let id = id.validate()?;

        match fs::remove_file(self.tile_path(id)).await {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(TileError::CacheIo(error)),
        }
    }

    pub fn tile_path(&self, id: TileId) -> PathBuf {
        self.root
            .join(id.z.to_string())
            .join(id.x.to_string())
            .join(format!("{}.tile", id.y))
    }

    fn temporary_tile_path(&self, id: TileId) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or_default();

        self.root
            .join(id.z.to_string())
            .join(id.x.to_string())
            .join(format!("{}.{}.tmp", id.y, nonce))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_cache_dir(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("osm-tile-engine-{}-{}", name, std::process::id()))
    }

    #[tokio::test]
    async fn stores_and_reads_tile_bytes() {
        let root = temp_cache_dir("stores-and-reads");
        let _ = fs::remove_dir_all(&root).await;

        let cache = FileTileCache::new(&root);
        let id = TileId::new(2, 1, 3).unwrap();
        let bytes = b"tile bytes";

        assert!(!cache.contains(id).await.unwrap());
        assert!(cache.get(id).await.unwrap().is_none());

        cache.put(id, bytes).await.unwrap();

        assert!(cache.contains(id).await.unwrap());
        assert_eq!(cache.get(id).await.unwrap(), Some(bytes.to_vec()));

        cache.remove(id).await.unwrap();
        assert!(!cache.contains(id).await.unwrap());

        let _ = fs::remove_dir_all(&root).await;
    }
}
