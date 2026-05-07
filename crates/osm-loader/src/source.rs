use async_trait::async_trait;
use reqwest::header::{ACCEPT, HeaderValue};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Mutex, Notify, futures::OwnedNotified};

use crate::{FileTileCache, TileError, TileId};

const TILE_USER_AGENT: &str = "osm-tile-engine/0.1";
const TILE_ACCEPT_HEADER: &str = "image/webp,image/png,image/jpeg,*/*;q=0.8";
const TILE_HTTP_CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
const TILE_HTTP_REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

#[async_trait]
pub trait TileSource: Send + Sync {
    async fn load_tile(&self, id: TileId) -> Result<Vec<u8>, TileError>;
}

#[derive(Debug, Clone)]
pub struct HttpTileSource {
    url_template: String,
    client: reqwest::Client,
}

impl HttpTileSource {
    pub fn new(url_template: impl Into<String>) -> Result<Self, TileError> {
        let url_template = url_template.into();

        if !url_template.contains("{z}")
            || !url_template.contains("{x}")
            || !url_template.contains("{y}")
        {
            return Err(TileError::InvalidTemplate);
        }

        Ok(Self {
            url_template,
            client: reqwest::Client::builder()
                .user_agent(TILE_USER_AGENT)
                .connect_timeout(TILE_HTTP_CONNECT_TIMEOUT)
                .timeout(TILE_HTTP_REQUEST_TIMEOUT)
                .build()?,
        })
    }

    pub fn url_template(&self) -> &str {
        &self.url_template
    }

    pub fn tile_url(&self, id: TileId) -> String {
        self.url_template
            .replace("{z}", &id.z.to_string())
            .replace("{x}", &id.x.to_string())
            .replace("{y}", &id.y.to_string())
    }
}

#[async_trait]
impl TileSource for HttpTileSource {
    async fn load_tile(&self, id: TileId) -> Result<Vec<u8>, TileError> {
        let id = id.validate()?;
        let response = self
            .client
            .get(self.tile_url(id))
            .header(ACCEPT, HeaderValue::from_static(TILE_ACCEPT_HEADER))
            .send()
            .await?;
        let status = response.status();

        if !status.is_success() {
            return Err(TileError::HttpStatus(status));
        }

        Ok(response.bytes().await?.to_vec())
    }
}

#[derive(Debug, Clone)]
pub struct CachedTileSource<S> {
    source: S,
    cache: FileTileCache,
    in_flight: Arc<Mutex<HashMap<TileId, Arc<Notify>>>>,
}

impl<S> CachedTileSource<S> {
    pub fn new(source: S, cache: FileTileCache) -> Self {
        Self {
            source,
            cache,
            in_flight: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn cache(&self) -> &FileTileCache {
        &self.cache
    }

    pub fn source(&self) -> &S {
        &self.source
    }
}

#[async_trait]
impl<S> TileSource for CachedTileSource<S>
where
    S: TileSource,
{
    async fn load_tile(&self, id: TileId) -> Result<Vec<u8>, TileError> {
        let id = id.validate()?;

        if let Some(bytes) = self.cache.get(id).await? {
            return Ok(bytes);
        }

        loop {
            let load_state = self.begin_tile_load(id).await;
            match load_state {
                TileLoadState::Leader(notify) => {
                    let result = async {
                        if let Some(bytes) = self.cache.get(id).await? {
                            return Ok(bytes);
                        }

                        let bytes = self.source.load_tile(id).await?;
                        self.cache.put(id, &bytes).await?;

                        Ok(bytes)
                    }
                    .await;

                    self.finish_tile_load(id, notify).await;
                    return result;
                }
                TileLoadState::Follower(notified) => {
                    notified.await;
                    if let Some(bytes) = self.cache.get(id).await? {
                        return Ok(bytes);
                    }
                }
            }
        }
    }
}

enum TileLoadState {
    Leader(Arc<Notify>),
    Follower(OwnedNotified),
}

impl<S> CachedTileSource<S> {
    async fn begin_tile_load(&self, id: TileId) -> TileLoadState {
        let mut in_flight = self.in_flight.lock().await;
        if let Some(notify) = in_flight.get(&id) {
            return TileLoadState::Follower(notify.clone().notified_owned());
        }

        let notify = Arc::new(Notify::new());
        in_flight.insert(id, notify.clone());
        TileLoadState::Leader(notify)
    }

    async fn finish_tile_load(&self, id: TileId, notify: Arc<Notify>) {
        self.in_flight.lock().await.remove(&id);
        notify.notify_waiters();
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    use tokio::fs;

    use super::*;

    #[derive(Debug)]
    struct CountingTileSource {
        calls: Arc<AtomicUsize>,
        bytes: Vec<u8>,
    }

    #[async_trait]
    impl TileSource for CountingTileSource {
        async fn load_tile(&self, _id: TileId) -> Result<Vec<u8>, TileError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(self.bytes.clone())
        }
    }

    #[derive(Debug)]
    struct SlowCountingTileSource {
        calls: Arc<AtomicUsize>,
        bytes: Vec<u8>,
        delay: Duration,
    }

    #[async_trait]
    impl TileSource for SlowCountingTileSource {
        async fn load_tile(&self, _id: TileId) -> Result<Vec<u8>, TileError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            std::thread::sleep(self.delay);
            Ok(self.bytes.clone())
        }
    }

    fn temp_cache_dir(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("osm-tile-engine-{}-{}", name, std::process::id()))
    }

    #[test]
    fn builds_tile_url_from_template() {
        let source = HttpTileSource::new("http://localhost/tile/{z}/{x}/{y}.png").unwrap();
        let id = TileId::new(12, 2382, 1305).unwrap();

        assert_eq!(
            source.tile_url(id),
            "http://localhost/tile/12/2382/1305.png"
        );
    }

    #[test]
    fn rejects_template_without_placeholders() {
        assert!(matches!(
            HttpTileSource::new("http://localhost/tile/{z}/{x}.png"),
            Err(TileError::InvalidTemplate)
        ));
    }

    #[tokio::test]
    async fn cached_source_returns_cache_hit_without_calling_source() {
        let root = temp_cache_dir("cache-hit");
        let _ = fs::remove_dir_all(&root).await;

        let cache = FileTileCache::new(&root);
        let id = TileId::new(3, 4, 5).unwrap();
        cache.put(id, b"cached tile").await.unwrap();

        let calls = Arc::new(AtomicUsize::new(0));
        let source = CountingTileSource {
            calls: calls.clone(),
            bytes: b"network tile".to_vec(),
        };
        let cached_source = CachedTileSource::new(source, cache);

        assert_eq!(
            cached_source.load_tile(id).await.unwrap(),
            b"cached tile".to_vec()
        );
        assert_eq!(calls.load(Ordering::SeqCst), 0);

        let _ = fs::remove_dir_all(&root).await;
    }

    #[tokio::test]
    async fn cached_source_downloads_and_stores_cache_miss() {
        let root = temp_cache_dir("cache-miss");
        let _ = fs::remove_dir_all(&root).await;

        let calls = Arc::new(AtomicUsize::new(0));
        let source = CountingTileSource {
            calls: calls.clone(),
            bytes: b"downloaded tile".to_vec(),
        };
        let cache = FileTileCache::new(&root);
        let cached_source = CachedTileSource::new(source, cache.clone());
        let id = TileId::new(4, 7, 9).unwrap();

        assert_eq!(
            cached_source.load_tile(id).await.unwrap(),
            b"downloaded tile".to_vec()
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            cache.get(id).await.unwrap(),
            Some(b"downloaded tile".to_vec())
        );

        let _ = fs::remove_dir_all(&root).await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn cached_source_coalesces_concurrent_misses_for_same_tile() {
        let root = temp_cache_dir("coalesced-miss");
        let _ = fs::remove_dir_all(&root).await;

        let calls = Arc::new(AtomicUsize::new(0));
        let source = SlowCountingTileSource {
            calls: calls.clone(),
            bytes: b"downloaded once".to_vec(),
            delay: Duration::from_millis(50),
        };
        let cache = FileTileCache::new(&root);
        let cached_source = Arc::new(CachedTileSource::new(source, cache));
        let id = TileId::new(6, 10, 11).unwrap();

        let mut handles = Vec::new();
        for _ in 0..8 {
            let cached_source = cached_source.clone();
            handles.push(tokio::spawn(async move {
                cached_source.load_tile(id).await.unwrap()
            }));
        }

        for handle in handles {
            assert_eq!(handle.await.unwrap(), b"downloaded once".to_vec());
        }
        assert_eq!(calls.load(Ordering::SeqCst), 1);

        let _ = fs::remove_dir_all(&root).await;
    }
}
