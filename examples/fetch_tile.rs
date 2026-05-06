use std::env;
use std::error::Error;
use std::path::PathBuf;

use osm_tile_core::{CachedTileSource, FileTileCache, HttpTileSource, TileId, TileSource};

const DEFAULT_URL_TEMPLATE: &str = "http://localhost:8080/tile/{z}/{x}/{y}.png";
const DEFAULT_CACHE_DIR: &str = "/tmp/osm-tile-cache";

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let config = Config::from_args(env::args().skip(1))?;
    let tile_id = TileId::new(config.z, config.x, config.y)?;

    let http_source = HttpTileSource::new(config.url_template)?;
    let cache = FileTileCache::new(config.cache_dir);
    let source = CachedTileSource::new(http_source, cache.clone());

    let was_cached = cache.contains(tile_id).await?;
    let bytes = source.load_tile(tile_id).await?;
    let is_cached = cache.contains(tile_id).await?;

    println!(
        "tile z={} x={} y={} loaded: {} bytes",
        tile_id.z,
        tile_id.x,
        tile_id.y,
        bytes.len()
    );
    println!("cache before request: {}", hit_miss(was_cached));
    println!("cache after request: {}", hit_miss(is_cached));
    println!("cache root: {}", cache.root().display());

    Ok(())
}

#[derive(Debug)]
struct Config {
    url_template: String,
    cache_dir: PathBuf,
    z: u32,
    x: u32,
    y: u32,
}

impl Config {
    fn from_args(args: impl IntoIterator<Item = String>) -> Result<Self, Box<dyn Error>> {
        let args = args.into_iter().collect::<Vec<_>>();

        match args.as_slice() {
            [] => Ok(Self {
                url_template: DEFAULT_URL_TEMPLATE.to_owned(),
                cache_dir: PathBuf::from(DEFAULT_CACHE_DIR),
                z: 0,
                x: 0,
                y: 0,
            }),
            [url_template, cache_dir, z, x, y] => Ok(Self {
                url_template: url_template.to_owned(),
                cache_dir: PathBuf::from(cache_dir),
                z: z.parse()?,
                x: x.parse()?,
                y: y.parse()?,
            }),
            _ => Err(usage().into()),
        }
    }
}

fn hit_miss(is_cached: bool) -> &'static str {
    if is_cached { "hit" } else { "miss" }
}

fn usage() -> &'static str {
    "usage: cargo run --example fetch_tile -- '<url-template>' <cache-dir> <z> <x> <y>"
}
