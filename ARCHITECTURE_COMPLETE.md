# MapLibre Native - Complete Architecture Analysis

> **Document Version:** 1.0
> **Codebase:** maplibre-native
> **Analysis Date:** May 2026

---

## Table of Contents

1. [Project Overview](#1-project-overview)
2. [Repository Structure](#2-repository-structure)
3. [High-Level Architecture](#3-high-level-architecture)
4. [Core Engine Components](#4-core-engine-components)
5. [Tile Rendering Engine](#5-tile-rendering-engine)
6. [Rendering Pipeline](#6-rendering-pipeline)
7. [Graphics Abstraction Layer](#7-graphics-abstraction-layer)
8. [Style System](#8-style-system)
9. [Threading Model](#9-threading-model)
10. [Actor Framework](#10-actor-framework)
11. [Platform SDKs](#11-platform-sdks)
12. [Build System](#12-build-system)
13. [Design Patterns](#13-design-patterns)
14. [Key Data Structures](#14-key-data-structures)
15. [Shader System](#15-shader-system)
16. [Text & Glyph System](#16-text--glyph-system)
17. [Layout System](#17-layout-system)
18. [Storage & Caching](#18-storage--caching)

---

## 1. Project Overview

MapLibre Native is a high-performance, cross-platform map rendering engine written in C++. It implements the [MapLibre Style Specification](https://maplibre.org/maplibre-style-spec/) and provides native rendering for mobile, desktop, and web platforms.

### Key Characteristics

- **Language:** C++17 core with platform-specific bindings (Kotlin/Java, Objective-C/Swift, JavaScript, C++)
- **Rendering Backends:** OpenGL ES, Vulkan, Metal, WebGPU
- **Tile Formats:** MVT (Mapbox Vector Tiles), MLT, GeoJSON, Raster, DEM
- **Platforms:** Android, iOS, macOS, Linux, Windows, Qt, Node.js
- **Architecture:** Layered, immutable style objects, actor-based concurrency

---

## 2. Repository Structure

```
maplibre-native/
├── ARCHITECTURE.md              # Official architecture documentation
├── CMakeLists.txt               # Main CMake build configuration
├── BUILD.bazel                  # Bazel build configuration
├── Makefile                     # Master Makefile coordinating builds
│
├── include/mbgl/                # Public C++ API headers (19 subdirectories)
│   ├── actor/                   # Actor framework (scheduler, mailbox, messages)
│   ├── annotation/              # Annotation types
│   ├── gfx/                     # Graphics abstraction (37 headers)
│   ├── gl/                      # OpenGL backend headers
│   ├── i18n/                    # Internationalization
│   ├── layermanager/            # Layer factory interfaces
│   ├── map/                     # Map API (Map, MapOptions, Camera)
│   ├── math/                    # Math utilities
│   ├── mtl/                     # Metal backend headers
│   ├── platform/                # Platform interfaces
│   ├── renderer/                # Renderer public API
│   ├── shaders/                 # Shader abstractions
│   ├── storage/                 # Storage interfaces
│   ├── style/                   # Style system (layers, sources, expressions)
│   ├── text/                    # Text/glyph interfaces
│   ├── tile/                    # Tile interfaces
│   ├── util/                    # Utility headers
│   ├── vulkan/                  # Vulkan backend headers
│   └── webgpu/                  # WebGPU backend headers
│
├── src/mbgl/                    # Private C++ implementation (24 subdirectories)
│   ├── actor/                   # Actor implementations
│   ├── algorithm/               # Algorithms
│   ├── annotation/              # Annotation implementations
│   ├── geometry/                # Geometry utilities
│   ├── gfx/                     # Graphics abstraction implementations
│   ├── gl/                      # OpenGL backend implementation
│   ├── layermanager/            # Layer factories
│   ├── layout/                  # Layout processing (symbol, line, pattern)
│   ├── map/                     # Map implementation
│   ├── math/                    # Math implementations
│   ├── mtl/                     # Metal backend implementation
│   ├── platform/                # Platform implementations
│   ├── plugin/                  # Plugin system
│   ├── programs/                # Shader program implementations
│   ├── renderer/                # Renderer implementation (61 files)
│   │   ├── buckets/             # Bucket implementations (18 files)
│   │   └── layers/              # Render layer implementations (54 files)
│   ├── shaders/                 # Shader code generation
│   ├── sprite/                  # Sprite handling
│   ├── storage/                 # Storage implementations (12 files)
│   ├── style/                   # Style implementations
│   ├── text/                    # Text/glyph implementations (34 files)
│   ├── tile/                    # Tile implementations (40 files)
│   ├── util/                    # Utility implementations
│   ├── vulkan/                  # Vulkan backend implementation
│   └── webgpu/                  # WebGPU backend implementation
│
├── platform/                    # Platform SDKs (13 subdirectories)
│   ├── android/                 # Android SDK (Kotlin/Java + JNI)
│   │   ├── MapLibreAndroid/     # Main library
│   │   ├── MapLibreAndroidTestApp/
│   │   ├── MapLibrePlugin/      # Gradle plugin
│   │   └── src/                 # JNI native code
│   ├── ios/                     # iOS SDK (Objective-C/Swift)
│   │   ├── framework/           # XCFramework build
│   │   ├── src/                 # Objective-C++ implementation
│   │   └── app/                 # Demo app
│   ├── macos/                   # macOS SDK
│   ├── qt/                      # Qt desktop SDK
│   ├── node/                    # Node.js bindings
│   ├── glfw/                    # GLFW test application
│   ├── darwin/                  # Shared Apple (iOS/macOS) code
│   ├── default/                 # Shared default implementations
│   ├── linux/                   # Linux platform support
│   └── windows/                 # Windows platform support
│
├── shaders/                     # GLSL shader sources (68 files)
│   ├── background.*.glsl        # Background layer shaders
│   ├── circle.*.glsl            # Circle layer shaders
│   ├── fill.*.glsl              # Fill layer shaders
│   ├── fill_extrusion.*.glsl    # 3D extrusion shaders
│   ├── line.*.glsl              # Line layer shaders
│   ├── raster.*.glsl            # Raster layer shaders
│   ├── symbol_*.glsl            # Symbol/text/icon shaders
│   ├── heatmap.*.glsl           # Heatmap shaders
│   ├── hillshade.*.glsl         # Hillshade shaders
│   ├── color_relief.*.glsl      # Color relief shaders
│   ├── debug.*.glsl             # Debug visualization shaders
│   └── collision_*.glsl         # Collision box shaders
│
├── test/                        # Unit tests for C++ core
├── render-test/                 # Render test infrastructure
├── benchmark/                   # Performance benchmarks
├── expression-test/             # Expression tests
├── metrics/                     # Render test ground truth images
├── vendor/                      # Third-party dependencies
├── misc/                        # Protobuf definitions, icons
├── bin/                         # CLI tools (mbgl-cache, mbgl-offline, mbgl-render)
├── docs/                        # Documentation (mdbook, doxygen)
├── design-proposals/            # Architecture design proposals
├── rustutils/                   # Rust utility components
└── cmake/                       # CMake helper modules
```

---

## 3. High-Level Architecture

### 3.1 Layered Architecture

```
┌─────────────────────────────────────────────────────────────────────┐
│                        Platform SDK Layer                           │
│  Android (Kotlin/JNI) │ iOS/macOS (Obj-C) │ Qt (C++) │ Node.js     │
└─────────────────────────────────────────────────────────────────────┘
                                  │
                                  ▼
┌─────────────────────────────────────────────────────────────────────┐
│                        Public C++ API Layer                         │
│  Map │ Style │ Layer │ Source │ Renderer │ MapObserver              │
└─────────────────────────────────────────────────────────────────────┘
                                  │
                                  ▼
┌─────────────────────────────────────────────────────────────────────┐
│                        Core Engine Layer                            │
│  RenderOrchestrator │ TilePyramid │ GeometryTileWorker │ Placement  │
└─────────────────────────────────────────────────────────────────────┘
                                  │
                                  ▼
┌─────────────────────────────────────────────────────────────────────┐
│                    Graphics Abstraction Layer                       │
│  Context │ Drawable │ CommandEncoder │ Texture2D │ VertexBuffer     │
└─────────────────────────────────────────────────────────────────────┘
                                  │
                                  ▼
┌─────────────────────────────────────────────────────────────────────┐
│                     Graphics Backend Layer                          │
│  OpenGL │ Vulkan │ Metal │ WebGPU                                  │
└─────────────────────────────────────────────────────────────────────┘
```

### 3.2 Data Flow Overview

```
Style JSON/URL
       │
       ▼
┌──────────────┐     ┌──────────────┐     ┌──────────────┐
│  Style Parser│────▶│ Style Objects│────▶│ Render Layers│
└──────────────┘     └──────────────┘     └──────────────┘
                                                  │
                                                  ▼
┌──────────────┐     ┌──────────────┐     ┌──────────────┐
│  Tile Server │────▶│  Tile Loader │────▶│ Tile Workers │
└──────────────┘     └──────────────┘     └──────────────┘
                                                  │
                                                  ▼
┌──────────────┐     ┌──────────────┐     ┌──────────────┐
│   GPU Render │◀────│  Buckets/    │◀────│ LayoutResult │
│              │     │  Drawables   │     │              │
└──────────────┘     └──────────────┘     └──────────────┘
```

---

## 4. Core Engine Components

### 4.1 Map Component

**File:** `include/mbgl/map/map.hpp:39`

The `Map` class is the primary entry point for controlling the map view.

```cpp
class Map : private util::noncopyable {
public:
    explicit Map(RendererFrontend&, MapObserver&, const MapOptions&,
                 const ResourceOptions&, const ClientOptions& = ClientOptions());
    ~Map();

    // Camera control
    void jumpTo(const CameraOptions&);
    void easeTo(const CameraOptions&, const AnimationOptions&);
    void flyTo(const CameraOptions&, const AnimationOptions&);

    // Style management
    style::Style& getStyle();
    void setStyle(std::unique_ptr<style::Style>);

    // Rendering
    void renderStill(StillImageCallback);
    void triggerRepaint();

    // Query
    std::vector<Feature> queryRenderedFeatures(...);
    std::vector<Feature> querySourceFeatures(...);
};
```

**Implementation:** `src/mbgl/map/map_impl.hpp`

Key responsibilities:
- Camera transformations (zoom, bearing, pitch, center)
- Style ownership and lifecycle
- Gesture handling coordination
- Tile prefetching with LOD control
- Bounds and projection management

### 4.2 Renderer Component

**File:** `include/mbgl/renderer/renderer.hpp:43`

```cpp
class Renderer {
public:
    Renderer(gfx::RendererBackend&, float pixelRatio_,
             const std::optional<std::string>& localFontFamily = std::nullopt);
    ~Renderer();

    void render(const std::shared_ptr<UpdateParameters>&);
    void setObserver(RendererObserver*);
    void markContextLost();

    // Feature queries
    std::vector<Feature> queryRenderedFeatures(...);
    std::vector<Feature> querySourceFeatures(...);

    // State management
    void setFeatureState(...);
    void reduceMemoryUse();
    void clearData();
};
```

**Implementation:** `src/mbgl/renderer/renderer_impl.hpp:27`

```cpp
class Renderer::Impl : public gfx::ContextObserver {
private:
    RenderOrchestrator orchestrator;      // Central render coordinator
    gfx::RendererBackend& backend;        // Platform rendering surface
    RendererObserver* observer;
    const float pixelRatio;
    std::unique_ptr<RenderStaticData> staticData;
    gfx::DynamicTextureAtlasPtr dynamicTextureAtlas;
};
```

### 4.3 RenderOrchestrator

**File:** `src/mbgl/renderer/render_orchestrator.hpp:54`

The central coordinator for all rendering operations.

```cpp
class RenderOrchestrator final : public GlyphManagerObserver,
                                  public ImageManagerObserver,
                                  public RenderSourceObserver {
public:
    // Render tree creation
    std::unique_ptr<RenderTree> createRenderTree(
        const std::shared_ptr<UpdateParameters>&, gfx::DynamicTextureAtlasPtr);

    // Layer group management
    bool addLayerGroup(LayerGroupBasePtr);
    bool removeLayerGroup(const LayerGroupBasePtr&);
    void visitLayerGroups(Func f);
    void visitLayerGroupsReversed(Func f);

    // Updates
    void update(const std::shared_ptr<UpdateParameters>&);
    void updateLayers(gfx::ShaderRegistry&, gfx::Context&,
                      const TransformState&, const std::shared_ptr<UpdateParameters>&,
                      const RenderTree&);

    // Managers
    std::shared_ptr<GlyphManager> glyphManager;
    std::shared_ptr<ImageManager> imageManager;
    std::unique_ptr<LineAtlas> lineAtlas;
    std::unique_ptr<PatternAtlas> patternAtlas;

    // State
    std::unordered_map<std::string, std::unique_ptr<RenderSource>> renderSources;
    std::unordered_map<std::string, std::unique_ptr<RenderLayer>> renderLayers;
    RenderLight renderLight;
    CrossTileSymbolIndex crossTileSymbolIndex;
    PlacementController placementController;
};
```

Key responsibilities:
- Style diffing to detect changes between frames
- Managing RenderSource and RenderLayer objects
- Creating RenderTree for each frame
- Glyph and image loading coordination
- Symbol placement and collision detection
- Layer group management for custom layers

---

## 5. Tile Rendering Engine

### 5.1 Tile Class Hierarchy

```
Tile (abstract base)
├── src/mbgl/tile/tile.hpp:50
│
├── GeometryTile
│   ├── src/mbgl/tile/geometry_tile.hpp:26
│   │   implements: GlyphRequestor, ImageRequestor
│   │
│   ├── VectorTile
│   │   ├── src/mbgl/tile/vector_tile.hpp:11
│   │   │
│   │   ├── VectorMVTTile (MVT format - Protocol Buffers)
│   │   │   └── src/mbgl/tile/vector_mvt_tile.hpp:12
│   │   │
│   │   └── VectorMLTTile (MLT format)
│   │       └── src/mbgl/tile/vector_mlt_tile.hpp:12
│   │
│   ├── GeoJSONTile
│   │   └── src/mbgl/tile/geojson_tile.hpp:14
│   │
│   ├── CustomGeometryTile
│   │   └── src/mbgl/tile/custom_geometry_tile.hpp:17
│   │
│   └── AnnotationTile
│       └── src/mbgl/annotation/annotation_tile.hpp:12
│
├── RasterTile
│   └── src/mbgl/tile/raster_tile.hpp:18
│
└── RasterDEMTile
    └── src/mbgl/tile/raster_dem_tile.hpp:62
```

### 5.2 Tile Base Class

**File:** `src/mbgl/tile/tile.hpp:50`

```cpp
class Tile : public TileLoaderObserver {
public:
    enum class Kind : uint8_t {
        Geometry,    // Vector tiles
        Raster,      // Image tiles
        RasterDEM    // Terrain/elevation tiles
    };

    // Lifecycle states
    bool isRenderable() const;   // Can be rendered (may be partial)
    bool isLoaded() const;       // Response received from FileSource
    bool isComplete() const;     // loaded && !pending

    // Tile identification
    const Kind kind;
    OverscaledTileID id;
    const std::string sourceID;

    // HTTP caching
    std::optional<Timestamp> modified;
    std::optional<Timestamp> expires;

    // Fade state for symbol transitions
    virtual bool holdForFade() const { return false; }
    virtual void markRenderedIdeal() {}
    virtual void markRenderedPreviously() {}
    virtual void performedFadePlacement() {}

protected:
    bool triedOptional = false;  // Cache attempted
    bool renderable = false;     // Can be rendered
    bool pending = false;        // Operations in progress
    bool loaded = false;         // Load completed
};
```

### 5.3 GeometryTile

**File:** `src/mbgl/tile/geometry_tile.hpp:26`

The most complex tile type, handling vector tile parsing, layout, and symbol placement.

```cpp
class GeometryTile : public Tile, public GlyphRequestor, public ImageRequestor {
public:
    const std::thread::id renderThreadID = std::this_thread::get_id();

    // Data management
    void setError(std::exception_ptr);
    void setData(std::unique_ptr<const GeometryTileData>);
    void reset();

    // Layout results
    void onLayout(std::shared_ptr<LayoutResult>, uint64_t correlationID);
    void onError(std::exception_ptr, uint64_t correlationID);

    // Glyph and image requests
    void onGlyphsAvailable(GlyphMap, HBShapeRequests) override;
    void onImagesAvailable(ImageMap, ImageMap, ImageVersionMap, uint64_t) override;
    void getGlyphs(GlyphDependencies);
    void getImages(ImageRequestPair);

    // Layout result structure
    class LayoutResult {
        mbgl::unordered_map<std::string, LayerRenderData> layerRenderData;
        std::shared_ptr<FeatureIndex> featureIndex;
        gfx::GlyphAtlas glyphAtlas;
        gfx::ImageAtlas imageAtlas;
        gfx::DynamicTextureAtlasPtr dynamicTextureAtlas;
    };

    // Fade states
    enum class FadeState {
        Loaded,
        NeedsFirstPlacement,
        NeedsSecondPlacement,
        CanRemove
    };

private:
    TaggedScheduler threadPool;
    const std::shared_ptr<Mailbox> mailbox;
    OptionalActor<GeometryTileWorker> worker;

    const std::shared_ptr<FileSource> fileSource;
    const std::shared_ptr<GlyphManager> glyphManager;
    const std::shared_ptr<ImageManager> imageManager;

    std::shared_ptr<LayoutResult> layoutResult;
    std::atomic<bool> obsolete{false};
};
```

### 5.4 GeometryTileWorker

**File:** `src/mbgl/tile/geometry_tile_worker.hpp:35`

Actor-based worker that processes tile data on background threads.

```cpp
class GeometryTileWorker {
public:
    GeometryTileWorker(OptionalActorRef<GeometryTileWorker> self,
                       OptionalActorRef<GeometryTile> parent,
                       const TaggedScheduler& scheduler_,
                       OverscaledTileID, std::string sourceID,
                       const std::atomic<bool>& obsolete,
                       MapMode, float pixelRatio,
                       bool showCollisionBoxes_,
                       gfx::DynamicTextureAtlasPtr,
                       std::shared_ptr<FontFaces> fontFaces);

    // Input methods (called from GeometryTile)
    void setLayers(std::vector<Immutable<style::LayerProperties>>,
                   std::set<std::string> availableImages, uint64_t correlationID);
    void setData(std::unique_ptr<const GeometryTileData>,
                 std::set<std::string> availableImages, uint64_t correlationID);
    void reset(uint64_t correlationID_);

    // Callback methods (called when dependencies resolved)
    void onGlyphsAvailable(GlyphMap glyphs, HBShapeResults requests);
    void onImagesAvailable(ImageMap newIconMap, ImageMap newPatternMap,
                           ImageVersionMap versionMap, uint64_t imageCorrelationID);

private:
    // State machine methods
    void coalesced();      // Process queued messages
    void parse();          // Parse features and create layouts
    void finalizeLayout(); // Complete layout and send result
    void coalesce();       // Merge pending changes

    void requestNewGlyphs(const GlyphDependencies&);
    void requestNewImages(const ImageDependencies&);
    void symbolDependenciesChanged();

    // State machine
    enum State {
        Idle,           // Ready for new input
        Coalescing,     // Merging pending changes
        NeedsParse,     // Waiting to parse
        NeedsSymbolLayout  // Waiting for glyphs/images
    };

    State state = Idle;

    // Data
    std::optional<std::vector<Immutable<style::LayerProperties>>> layers;
    std::optional<std::unique_ptr<const GeometryTileData>> data;
    std::vector<std::unique_ptr<Layout>> layouts;

    // Dependencies
    GlyphDependencies pendingGlyphDependencies;
    ImageDependencies pendingImageDependencies;
    GlyphMap glyphMap;
    ImageMap iconMap;
    ImageMap patternMap;

    // Results
    std::unique_ptr<FeatureIndex> featureIndex;
    mbgl::unordered_map<std::string, LayerRenderData> renderData;
};
```

### 5.5 Tile Worker State Machine

```
                    ┌─────────────────────────────────────────────┐
                    │                                             │
                    ▼                                             │
┌─────────┐    ┌──────────────┐    ┌─────────────┐    ┌──────────────────┐
│  Idle   │───▶│  Coalescing  │───▶│ NeedsParse  │───▶│  FinalizeLayout  │
│         │◀───│              │    │             │    │                  │
└─────────┘    └──────────────┘    └──────┬──────┘    └────────┬─────────┘
     ▲                                    │                    │
     │                                    ▼                    │
     │                           ┌─────────────────┐           │
     │                           │NeedsSymbolLayout│───────────┘
     │                           │                 │ (glyphs/images ready)
     │                           └─────────────────┘
     │                                    │
     │                                    ▼
     │                           ┌─────────────┐
     └───────────────────────────│ NeedsParse  │
                                 └─────────────┘

Transitions:
- setData() / setLayers(): Idle -> Coalescing -> NeedsParse
- Missing glyphs/images: NeedsParse -> NeedsSymbolLayout
- Dependencies resolved: NeedsSymbolLayout -> NeedsParse
- Parse complete: NeedsParse -> FinalizeLayout -> Idle
```

### 5.6 Tile Data Hierarchy

```
GeometryTileData (abstract)
├── src/mbgl/tile/geometry_tile_data.hpp:85
│
├── VectorMVTTileData
│   └── src/mbgl/tile/vector_mvt_tile_data.hpp:52
│       Parses MVT (Protocol Buffer) format
│
├── VectorMLTTileData
│   └── src/mbgl/tile/vector_mlt_tile_data.hpp:69
│       Parses MLT format
│
└── GeoJSONTileData
    └── src/mbgl/tile/geojson_tile_data.hpp:64
        Parses GeoJSON data
```

### 5.7 TileLoader

**File:** `src/mbgl/tile/tile_loader.hpp:19`

Handles network and cache operations for tile data.

```cpp
// Key operations:
// 1. makeRequired() - Initiate network fetch for needed tiles
// 2. makeOptional() - Reduce priority for non-visible tiles
// 3. loadFromCache() - Check local cache first
// 4. loadFromNetwork() - Fetch from tile server
// 5. loadedData() - Process response and parse tile
```

### 5.8 TilePyramid

**File:** `src/mbgl/renderer/tile_pyramid.hpp:30`

Manages collection of tiles at multiple zoom levels.

```cpp
class TilePyramid {
public:
    TilePyramid(const TaggedScheduler& threadPool_);
    ~TilePyramid();

    // Update tiles based on current viewport
    void update(const std::vector<Immutable<style::LayerProperties>>& visibleLayers,
                bool needsRendering, bool needsRelayout,
                const TileParameters&, const style::Source::Impl&,
                uint16_t tileSize, Range<uint8_t> zoomRange,
                std::optional<LatLngBounds> bounds,
                std::function<std::unique_ptr<Tile>(const OverscaledTileID&, TileObserver*)> createTile);

    // Tile access
    const std::map<UnwrappedTileID, std::reference_wrapper<Tile>>& getRenderedTiles() const;
    Tile* getTile(const OverscaledTileID&);
    const Tile* getRenderedTile(const UnwrappedTileID&) const;

    // Cache management
    void setCacheEnabled(bool);
    void reduceMemoryUse();

    // Fading support
    void updateFadingTiles();
    bool hasFadingTiles() const;

private:
    std::map<OverscaledTileID, std::unique_ptr<Tile>> tiles;
    TileCache cache;
    std::map<UnwrappedTileID, std::reference_wrapper<Tile>> renderedTiles;
    bool fadingTiles = false;
    bool cacheEnabled = true;
};
```

### 5.9 TileCache

**File:** `src/mbgl/tile/tile_cache.hpp:16`

LRU cache for tile data.

```cpp
class TileCache {
public:
    TileCache(size_t sizeLimit = 0);

    void add(const OverscaledTileID&, std::unique_ptr<Tile>);
    std::unique_ptr<Tile> get(const OverscaledTileID&);
    void clear();
    void setSize(size_t sizeLimit);
    size_t size() const;
};
```

---

## 6. Rendering Pipeline

### 6.1 Complete Data Flow: Tile Download to Screen

```
┌──────────────────────────────────────────────────────────────────────┐
│                        TILE DOWNLOAD PHASE                           │
├──────────────────────────────────────────────────────────────────────┤
│                                                                      │
│  [Tile Server]                                                       │
│       │                                                              │
│       ▼                                                              │
│  [FileSource] ──── HTTP/Local/Offline/PMTiles file requests          │
│       │                                                              │
│       ▼                                                              │
│  [TileLoader] ──── Downloads tile data, handles caching              │
│       │                                                              │
│       ▼                                                              │
│  [TileParser] ──── Parses MVT/MLT/GeoJSON into GeometryTileData      │
│                                                                      │
└──────────────────────────────────────────────────────────────────────┘
                                  │
                                  ▼
┌──────────────────────────────────────────────────────────────────────┐
│                       LAYOUT PROCESSING PHASE                        │
├──────────────────────────────────────────────────────────────────────┤
│                                                                      │
│  [GeometryTileWorker] (Background Thread)                            │
│       │                                                              │
│       ├── Parse features from tile data                              │
│       ├── Create Layout objects per layer                            │
│       ├── Request glyphs from GlyphManager                           │
│       ├── Request images from ImageManager                           │
│       ├── Create Buckets per layer                                   │
│       └── Generate LayoutResult                                      │
│                                                                      │
│  [LayoutResult]                                                      │
│       ├── LayerRenderData (per layer)                                │
│       ├── FeatureIndex (for queries)                                 │
│       ├── GlyphAtlas                                                 │
│       ├── ImageAtlas                                                 │
│       └── DynamicTextureAtlas                                        │
│                                                                      │
└──────────────────────────────────────────────────────────────────────┘
                                  │
                                  ▼
┌──────────────────────────────────────────────────────────────────────┐
│                      RENDER PREPARATION PHASE                        │
├──────────────────────────────────────────────────────────────────────┤
│                                                                      │
│  [GeometryTile] ──── Receives LayoutResult, creates TileRenderData   │
│       │                                                              │
│       ▼                                                              │
│  [TilePyramid] ──── Manages tile collection, caching, fading         │
│       │                                                              │
│       ▼                                                              │
│  [RenderSource] ──── RenderTileSource hierarchy                      │
│       │                                                              │
│       ▼                                                              │
│  [RenderOrchestrator] ──── Creates RenderTree                        │
│       │                                                              │
│       ├── Style diffing (detect changes)                             │
│       ├── Update RenderLayers                                        │
│       ├── Symbol placement (PlacementController)                     │
│       ├── Collision detection (CrossTileSymbolIndex)                 │
│       └── Build RenderTree with RenderItems                          │
│                                                                      │
└──────────────────────────────────────────────────────────────────────┘
                                  │
                                  ▼
┌──────────────────────────────────────────────────────────────────────┐
│                        GPU RENDERING PHASE                           │
├──────────────────────────────────────────────────────────────────────┤
│                                                                      │
│  [Renderer::Impl::render()] ──── Frame rendering loop                │
│       │                                                              │
│       ▼                                                              │
│  [gfx::Context] ──── Graphics context (backend-agnostic)             │
│       │                                                              │
│       ├── beginFrame()                                               │
│       ├── Create CommandEncoder                                      │
│       ├── Process RenderTree                                         │
│       │   ├── For each RenderItem:                                   │
│       │   │   ├── Apply LayerTweaker (per-frame updates)             │
│       │   │   ├── Update uniforms (UBOs)                             │
│       │   │   └── Draw Drawables                                     │
│       │   └── For each TileLayerGroup:                               │
│       │       ├── Bind shader                                        │
│       │       ├── Bind textures                                      │
│       │       ├── Bind vertex buffers                                │
│       │       └── Execute draw commands                              │
│       └── endFrame()                                                 │
│                                                                      │
│  [Drawable] ──── GPU draw commands via backend                       │
│       │                                                              │
│       ▼                                                              │
│  [Graphics Backend] ──── OpenGL / Vulkan / Metal / WebGPU            │
│       │                                                              │
│       ▼                                                              │
│  [Screen]                                                            │
│                                                                      │
└──────────────────────────────────────────────────────────────────────┘
```

### 6.2 RenderTree Structure

```
RenderTree
├── src/mbgl/renderer/render_tree.hpp
│
├── RenderItem (base)
│   ├── getDrawables() ──── Returns drawables for rendering
│   ├── getLayerTweaker() ──── Per-frame update logic
│   │
│   ├── TileSourceRenderItem
│   │   └── src/mbgl/renderer/sources/render_tile_source.hpp:68
│   │       Renders tiles from a tile source
│   │
│   └── LayerGroup
│       └── include/mbgl/renderer/layer_group.hpp
│           Groups layers for efficient rendering
```

### 6.3 Render Layers

Located in `src/mbgl/renderer/layers/`:

| Layer Class | File | Purpose |
|-------------|------|---------|
| RenderBackgroundLayer | `render_background_layer.hpp` | Background color/pattern |
| RenderCircleLayer | `render_circle_layer.hpp` | Circle markers |
| RenderFillLayer | `render_fill_layer.hpp` | Polygon fills |
| RenderFillExtrusionLayer | `render_fill_extrusion_layer.hpp` | 3D building extrusions |
| RenderHeatmapLayer | `render_heatmap_layer.hpp` | Heatmap visualization |
| RenderHillshadeLayer | `render_hillshade_layer.hpp` | Terrain hillshading |
| RenderColorReliefLayer | `render_color_relief_layer.hpp` | DEM color relief |
| RenderLineLayer | `render_line_layer.hpp` | Line features (roads, rivers) |
| RenderRasterLayer | `render_raster_layer.hpp` | Raster image tiles |
| RenderSymbolLayer | `render_symbol_layer.hpp` | Text labels and icons |
| RenderLocationIndicatorLayer | `render_location_indicator_layer.hpp` | Location indicator |
| RenderCustomLayer | `render_custom_layer.hpp` | User-provided OpenGL rendering |
| RenderCustomDrawableLayer | `render_custom_drawable_layer.hpp` | User-provided Drawable rendering |

### 6.4 Layer Tweakers

Per-frame update logic for each layer type:

| Tweaker | File | Purpose |
|---------|------|---------|
| BackgroundLayerTweaker | `background_layer_tweaker.hpp` | Background updates |
| CircleLayerTweaker | `circle_layer_tweaker.hpp` | Circle property updates |
| FillLayerTweaker | `fill_layer_tweaker.hpp` | Fill property updates |
| FillExtrusionLayerTweaker | `fill_extrusion_layer_tweaker.hpp` | 3D extrusion updates |
| HeatmapLayerTweaker | `heatmap_layer_tweaker.hpp` | Heatmap updates |
| HillshadeLayerTweaker | `hillshade_layer_tweaker.hpp` | Hillshade updates |
| LineLayerTweaker | `line_layer_tweaker.hpp` | Line property updates |
| RasterLayerTweaker | `raster_layer_tweaker.hpp` | Raster updates |
| SymbolLayerTweaker | `symbol_layer_tweaker.hpp` | Symbol opacity/placement updates |
| CollisionLayerTweaker | `collision_layer_tweaker.hpp` | Collision box visualization |

### 6.5 Bucket System

Located in `src/mbgl/renderer/buckets/`:

Buckets are GPU-ready data containers for features of a specific layer type.

| Bucket | File | Purpose |
|--------|------|---------|
| CircleBucket | `circle_bucket.hpp` | Circle feature geometry |
| FillBucket | `fill_bucket.hpp` | Polygon fill geometry |
| FillExtrusionBucket | `fill_extrusion_bucket.hpp` | 3D extrusion geometry |
| HeatmapBucket | `heatmap_bucket.hpp` | Heatmap point data |
| HillshadeBucket | `hillshade_bucket.hpp` | Hillshade tile data |
| LineBucket | `line_bucket.hpp` | Line feature geometry |
| RasterBucket | `raster_bucket.hpp` | Raster tile data |
| SymbolBucket | `symbol_bucket.hpp` | Text and icon symbol data |
| DebugBucket | `debug_bucket.hpp` | Debug visualization |

**Bucket Base Class:** `src/mbgl/renderer/bucket.hpp:28`

```cpp
class Bucket {
public:
    // Add a feature to the bucket
    virtual void addFeature(const GeometryTileFeature&,
                            const GeometryCollection&,
                            const ImagePositions&,
                            const PatternLayerMap&,
                            std::size_t,
                            const CanonicalTileID&);

    // Upload data to GPU
    virtual void upload(gfx::UploadPass&) = 0;

    // Check if bucket has renderable data
    virtual bool hasData() const = 0;

    // Cross-tile symbol placement
    virtual std::pair<uint32_t, bool> registerAtCrossTileIndex(...);
    virtual void place(Placement&, const BucketPlacementData&, std::set<uint32_t>&);
    virtual void updateVertices(const Placement&, bool, const TransformState&,
                                const RenderTile&, std::set<uint32_t>&);
};
```

### 6.6 Drawable System

**File:** `include/mbgl/gfx/drawable.hpp:52`

The fundamental unit of rendering - encapsulates everything needed to draw geometry.

```cpp
class Drawable {
public:
    // Execute draw command
    virtual void draw(PaintParameters&) const = 0;

    // Shader
    const gfx::ShaderProgramBasePtr& getShader() const;
    virtual void setShader(gfx::ShaderProgramBasePtr value);

    // Render pass (Opaque, Transparent, etc.)
    mbgl::RenderPass getRenderPass() const;
    bool hasRenderPass(const mbgl::RenderPass value) const;

    // Textures
    const gfx::Texture2DPtr& getTexture(size_t id) const;
    void setTextures(const Textures& textures_);

    // State toggles
    bool getEnabled() const;
    bool getEnableColor() const;
    bool getEnableStencil() const;
    bool getEnableDepth() const;
    bool getIs3D() const;
    bool getIsCustom() const;

    // Depth management
    DrawPriority getDrawPriority() const;
    int32_t getSubLayerIndex() const;
    DepthMaskType getDepthType() const;

    // Vertex data
    const gfx::VertexAttributeArrayPtr& getVertexAttributes() const;
    virtual void updateVertexAttributes(...);
    virtual void setVertices(std::vector<uint8_t>&&, std::size_t, AttributeDataType);

    // Index data
    void setIndexData(std::vector<std::uint16_t> indexes, std::vector<UniqueDrawSegment>);

    // Per-frame updates via tweakers
    const std::vector<DrawableTweakerPtr>& getTweakers() const;
    void addTweaker(DrawableTweakerPtr value);

    // Uniform buffers (UBOs)
    virtual const gfx::UniformBufferArray& getUniformBuffers() const;
    virtual gfx::UniformBufferArray& mutableUniformBuffers();

    // Tile association
    const std::optional<OverscaledTileID>& getTileID() const;
    const RenderTile* getRenderTile() const;
};
```

---

## 7. Graphics Abstraction Layer

### 7.1 Backend Types

**File:** `include/mbgl/gfx/backend.hpp:10`

```cpp
class Backend {
public:
    enum class Type : uint8_t {
        OpenGL,   ///< OpenGL ES / Desktop OpenGL
        Metal,    ///< Apple Metal
        Vulkan,   ///< Khronos Vulkan
        WebGPU,   ///< WebGPU
        TYPE_MAX  ///< Not a valid backend
    };

    // Runtime backend selection
    static void SetType(const Type value);
    static Type GetType();

    // Factory pattern for backend-specific objects
    template <typename T, typename... Args>
    static std::unique_ptr<T> Create(Args... args);
};
```

### 7.2 Backend Implementations

| Backend | Headers | Source | Status |
|---------|---------|--------|--------|
| **OpenGL** | `include/mbgl/gl/` (10 files) | `src/mbgl/gl/` | Stable, default |
| **Vulkan** | `include/mbgl/vulkan/` (20 files) | `src/mbgl/vulkan/` | Production |
| **Metal** | `include/mbgl/mtl/` (19 files) | `src/mbgl/mtl/` | Production (Apple) |
| **WebGPU** | `include/mbgl/webgpu/` | `src/mbgl/webgpu/` | Production |

### 7.3 Graphics Context

**File:** `include/mbgl/gfx/context.hpp:57`

Abstract graphics context - the central interface for all GPU operations.

```cpp
class Context {
public:
    // Frame management
    virtual void beginFrame() = 0;
    virtual void endFrame() = 0;
    virtual void performCleanup() = 0;
    virtual void reduceMemoryUsage() = 0;

    // Resource creation
    virtual std::unique_ptr<OffscreenTexture> createOffscreenTexture(Size, TextureChannelDataType);
    virtual std::unique_ptr<CommandEncoder> createCommandEncoder() = 0;
    virtual UniqueDrawableBuilder createDrawableBuilder(std::string name) = 0;
    virtual UniformBufferPtr createUniformBuffer(const void* data, std::size_t size, bool persistent);
    virtual Texture2DPtr createTexture2D() = 0;
    virtual DynamicTexturePtr createDynamicTexture(Size, TexturePixelType);
    virtual RenderTargetPtr createRenderTarget(const Size, const TextureChannelDataType);
    virtual gfx::VertexAttributeArrayPtr createVertexAttributeArray() const = 0;

    // Layer groups
    virtual TileLayerGroupPtr createTileLayerGroup(int32_t layerIndex, std::size_t capacity, std::string name);
    virtual LayerGroupPtr createLayerGroup(int32_t layerIndex, std::size_t capacity, std::string name);

    // Uniform buffers
    virtual bool emplaceOrUpdateUniformBuffer(gfx::UniformBufferPtr&, const void* data, std::size_t size);
    virtual const gfx::UniformBufferArray& getGlobalUniformBuffers() const = 0;
    virtual void bindGlobalUniformBuffers(gfx::RenderPass&) const noexcept = 0;

    // State management
    virtual void resetState(gfx::DepthMode, gfx::ColorMode) = 0;
    virtual void setDirtyState() = 0;
    virtual void clearStencilBuffer(int32_t) = 0;

    // Stats
    gfx::RenderingStats& renderingStats();
};
```

### 7.4 Key Graphics Abstractions

| Abstraction | File | Purpose |
|-------------|------|---------|
| **Drawable** | `include/mbgl/gfx/drawable.hpp:52` | Renderable geometry unit |
| **DrawableBuilder** | `include/mbgl/gfx/drawable_builder.hpp` | Constructs Drawables |
| **DrawableTweaker** | `include/mbgl/gfx/drawable_tweaker.hpp` | Per-frame Drawable updates |
| **Texture2D** | `include/mbgl/gfx/texture2d.hpp` | 2D texture abstraction |
| **DynamicTexture** | `include/mbgl/gfx/dynamic_texture.hpp` | Updatable texture |
| **DynamicTextureAtlas** | `include/mbgl/gfx/dynamic_texture_atlas.hpp` | Texture atlas for symbols |
| **VertexBuffer** | `include/mbgl/gfx/vertex_buffer.hpp` | Vertex data storage |
| **UniformBuffer** | `include/mbgl/gfx/uniform_buffer.hpp` | Uniform data for shaders |
| **CommandEncoder** | `include/mbgl/gfx/command_encoder.hpp` | GPU command recording |
| **RenderPass** | `include/mbgl/gfx/render_pass.hpp` | Render pass abstraction |
| **DrawScope** | `include/mbgl/gfx/draw_scope.hpp` | Draw state scope |
| **Renderbuffer** | `include/mbgl/gfx/renderbuffer.hpp` | Offscreen render target |

### 7.5 Backend-Specific Components

**OpenGL Backend (`include/mbgl/gl/`):**
- `drawable_gl.hpp` - OpenGL Drawable implementation
- `drawable_gl_builder.hpp` - OpenGL Drawable builder
- `renderer_backend.hpp` - OpenGL surface management
- `texture2d.hpp` - OpenGL texture
- `uniform_buffer_gl.hpp` - OpenGL UBO
- `vertex_attribute_gl.hpp` - OpenGL vertex attributes

**Vulkan Backend (`include/mbgl/vulkan/`):**
- `context.hpp` - Vulkan context
- `drawable.hpp` - Vulkan Drawable
- `drawable_builder.hpp` - Vulkan builder
- `command_encoder.hpp` - Vulkan command recording
- `render_pass.hpp` - Vulkan render pass
- `pipeline.hpp` - Vulkan pipeline
- `descriptor_set.hpp` - Vulkan descriptors
- `tile_layer_group.hpp` - Vulkan tile layer group

**Metal Backend (`include/mbgl/mtl/`):**
- `context.hpp` - Metal context
- `drawable.hpp` - Metal Drawable
- `drawable_builder.hpp` - Metal builder
- `command_encoder.hpp` - Metal command encoding
- `render_pass.hpp` - Metal render pass
- `tile_layer_group.hpp` - Metal tile layer group

---

## 8. Style System

### 8.1 Style Class

**File:** `include/mbgl/style/style.hpp:24`

```cpp
class Style {
public:
    // Layer management
    void addLayer(std::unique_ptr<Layer>);
    void addLayerBelow(std::unique_ptr<Layer>, const std::string& below);
    void removeLayer(const Layer&);
    void removeLayer(const std::string& id);

    // Source management
    void addSource(std::unique_ptr<Source>);
    void removeSource(const Source&);
    void removeSource(const std::string& id);

    // Image management
    void addImage(std::unique_ptr<Image>);
    void removeImage(const std::string& name);

    // Light
    void setLight(std::unique_ptr<Light>);

    // Access
    Layer* getLayer(const std::string& id);
    Source* getSource(const std::string& id);
};
```

### 8.2 Layer Types

Located in `include/mbgl/style/layers/`:

| Layer | File | Style Spec Property |
|-------|------|---------------------|
| BackgroundLayer | `background_layer.hpp` | background-color, background-pattern |
| CircleLayer | `circle_layer.hpp` | circle-radius, circle-color, circle-opacity |
| FillLayer | `fill_layer.hpp` | fill-color, fill-opacity, fill-pattern |
| FillExtrusionLayer | `fill_extrusion_layer.hpp` | fill-extrusion-height, fill-extrusion-color |
| HeatmapLayer | `heatmap_layer.hpp` | heatmap-weight, heatmap-color |
| HillshadeLayer | `hillshade_layer.hpp` | hillshade-illumination-direction |
| ColorReliefLayer | `color_relief_layer.hpp` | Color relief from DEM |
| LineLayer | `line_layer.hpp` | line-color, line-width, line-dasharray |
| LocationIndicatorLayer | `location_indicator_layer.hpp` | Location indicator |
| RasterLayer | `raster_layer.hpp` | raster-opacity, raster-hue-rotate |
| SymbolLayer | `symbol_layer.hpp` | text-field, icon-image, text-size |
| CustomLayer | `custom_layer.hpp` | User-provided rendering |
| CustomDrawableLayer | `custom_drawable_layer.hpp` | User-provided Drawable |

### 8.3 Source Types

Located in `include/mbgl/style/sources/`:

| Source | File | Purpose |
|--------|------|---------|
| VectorSource | `vector_source.hpp` | Vector tile source (MVT) |
| RasterSource | `raster_source.hpp` | Raster tile source |
| RasterDEMSource | `raster_dem_source.hpp` | Terrain/DEM source |
| GeoJSONSource | `geojson_source.hpp` | GeoJSON data source |
| ImageSource | `image_source.hpp` | Image overlay source |
| CustomGeometrySource | `custom_geometry_source.hpp` | Custom geometry source |
| TileSource | `tile_source.hpp` | Base tile source class |

### 8.4 Layer::Impl Pattern

Each layer has a parallel Impl class hierarchy for immutability:

```
Layer (mutable, public API)
├── impl() -> Immutable<Layer::Impl>
│
├── CircleLayer
│   └── impl() -> Immutable<CircleLayer::Impl>
│
├── FillLayer
│   └── impl() -> Immutable<FillLayer::Impl>
│
└── SymbolLayer
    └── impl() -> Immutable<SymbolLayer::Impl>
```

**Layer Properties:** `include/mbgl/style/layer_properties.hpp`

```cpp
class LayerProperties {
    Immutable<style::Layer::Impl> layerImpl;
    // Immutable reference to layer implementation
};
```

### 8.5 Expression System

Located in `include/mbgl/style/expression/`:

Full implementation of MapLibre Style Spec expressions:
- `interpolate` - Interpolate between values
- `step` - Step function
- `match` - Match expression
- `case` - Conditional expression
- `coalesce` - First non-null value
- `get`, `has` - Feature property access
- `feature-state` - Dynamic feature state
- GPU expression evaluation support (`gpu_expression.hpp`)

---

## 9. Threading Model

### 9.1 Thread Architecture

```
┌─────────────────────────────────────────────────────────────────────┐
│                        Main Thread                                   │
│                                                                      │
│  - Map API handling                                                  │
│  - Style ownership and mutations                                     │
│  - UI event dispatch                                                 │
│  - RenderTree creation (iOS)                                         │
│  - Camera transformations                                            │
└─────────────────────────────────────────────────────────────────────┘
         │                              │                    │
         │ Actor messages               │ File requests      │ Render calls
         ▼                              ▼                    ▼
┌─────────────────────┐  ┌─────────────────────┐  ┌─────────────────────┐
│   Worker Threads     │  │  FileSource Thread  │  │   Render Thread     │
│   (4 per Style)     │  │                     │  │   (Android)         │
│                     │  │  - Network requests │  │                     │
│  - Tile parsing     │  │  - Offline DB I/O   │  │  - GPU commands     │
│  - Layout compute   │  │  - Style fetching   │  │  - Frame rendering  │
│  - Bucket creation  │  │  - Cache operations │  │  - Drawable draws   │
│  - Glyph requests   │  │                     │  │  - Context ops      │
└─────────────────────┘  └─────────────────────┘  └─────────────────────┘
```

### 9.2 Thread Responsibilities

| Thread | Purpose | Key Classes |
|--------|---------|-------------|
| **Main Thread** | Map API, Style ownership, UI events | Map, Style, Layer, Source |
| **Worker Threads** | Tile parsing, layout computation | GeometryTileWorker, RasterTileWorker |
| **FileSource Thread** | Network requests, offline DB | FileSource, HTTPFileSource |
| **Render Thread** | Frame rendering (Android) | Renderer, Context, Drawable |

---

## 10. Actor Framework

### 10.1 Components

Located in `include/mbgl/actor/`:

| Component | File | Purpose |
|-----------|------|---------|
| **Scheduler** | `scheduler.hpp:37` | Task scheduling interface |
| **TaggedScheduler** | `scheduler.hpp:128` | Scheduler with owner tag |
| **Mailbox** | `mailbox.hpp` | Message queue for actors |
| **Actor<T>** | `actor.hpp` | Thread-safe actor wrapper |
| **ActorRef<T>** | `actor_ref.hpp` | Reference to communicate with actor |
| **OptionalActor<T>** | `optional_actor.hpp` | Actor that may not exist |
| **OptionalActorRef<T>** | `optional_actor_ref.hpp` | Optional actor reference |
| **Message<T>** | `message.hpp` | Message container |
| **AspiringActor** | `aspiring_actor.hpp` | Actor before mailbox assigned |
| **EstablishedActor** | `established_actor.hpp` | Actor with mailbox |

### 10.2 Scheduler Interface

**File:** `include/mbgl/actor/scheduler.hpp:37`

```cpp
class Scheduler {
public:
    virtual ~Scheduler() = default;

    // Enqueue function for execution
    virtual void schedule(std::function<void()>&&) = 0;
    virtual void schedule(const util::SimpleIdentity, std::function<void()>&&) = 0;

    // Render thread scheduling
    virtual void runOnRenderThread(const util::SimpleIdentity, std::function<void()>&&);
    virtual void runRenderJobs(const util::SimpleIdentity tag, bool closeQueue);

    // Wait for completion
    virtual void waitForEmpty(const util::SimpleIdentity = util::SimpleIdentity::Empty) = 0;

    // Global schedulers
    static Scheduler* GetCurrent(bool init = true);
    static void SetCurrent(Scheduler*);
    [[nodiscard]] static std::shared_ptr<Scheduler> GetBackground();
    [[nodiscard]] static std::shared_ptr<Scheduler> GetSequenced();
};
```

### 10.3 Actor Usage Pattern

```cpp
// Create worker on background thread
Actor<GeometryTileWorker> worker(
    threadPool,                              // Scheduler
    ActorRef<GeometryTile>(parent, mailbox)  // Parent reference
);

// Send message to actor
worker.invoke(&GeometryTileWorker::setData, std::move(tileData), images, correlationID);

// Actor processes message on its thread
// Results sent back via ActorRef
```

---

## 11. Platform SDKs

### 11.1 Platform Structure

```
platform/
├── android/                    # Android SDK
│   ├── MapLibreAndroid/        # Main library (Kotlin/Java)
│   │   ├── src/main/java/org/maplibre/android/
│   │   │   ├── MapView.java
│   │   │   ├── MapLibreMap.java
│   │   │   ├── style/
│   │   │   ├── location/
│   │   │   └── ...
│   │   └── src/main/cpp/       # JNI native code
│   ├── MapLibreAndroidTestApp/ # Test application
│   └── MapLibrePlugin/         # Gradle plugin
│
├── ios/                        # iOS SDK
│   ├── framework/              # XCFramework build
│   ├── src/                    # Objective-C++ implementation
│   │   ├── MGLMapView.h/mm
│   │   ├── MGLMapView+Impl.mm
│   │   └── ...
│   └── app/                    # Demo app
│
├── macos/                      # macOS SDK
│   ├── framework/
│   └── src/
│
├── qt/                         # Qt desktop SDK
│   └── src/
│       ├── qquickmaplibre.cpp
│       └── ...
│
├── node/                       # Node.js bindings
│   └── src/
│       ├── node_map.cpp
│       └── ...
│
├── glfw/                       # GLFW test application
│   └── src/
│       ├── glfw_view.cpp
│       └── ...
│
├── darwin/                     # Shared Apple (iOS/macOS) code
│   ├── core/                   # Core Objective-C++ bridge
│   └── src/
│
├── default/                    # Default platform implementations
│   └── src/                    # RunLoop, FileSource, etc.
│
├── linux/                      # Linux platform support
│
└── windows/                    # Windows platform support
    └── vendor/vcpkg/           # vcpkg dependency manager
```

### 11.2 Platform Interfaces

**RendererBackend:** `include/mbgl/gfx/renderer_backend.hpp`

Platform-specific rendering surface abstraction.

### 11.3 JNI Bridge (Android)

```
Java (Kotlin)                    C++ (JNI)
─────────────                   ─────────
MapView.java                    native_map.cpp
    │                               │
    ├── nativeCreate() ─────────▶ Map::create()
    ├── nativeRender() ─────────▶ Renderer::render()
    ├── nativeSetStyleUrl() ────▶ Style::loadURL()
    └── nativeQueryFeatures() ──▶ Renderer::queryRenderedFeatures()
```

---

## 12. Build System

### 12.1 CMake Options

**File:** `CMakeLists.txt`

| Option | Default | Description |
|--------|---------|-------------|
| `MLN_WITH_CORE_ONLY` | OFF | Build only core library |
| `MLN_WITH_QT` | OFF | Build Qt bindings |
| `MLN_WITH_NODE` | OFF | Build Node.js bindings |
| `MLN_WITH_GLFW` | ON | Build GLFW test app |
| `MLN_WITH_OPENGL` | OFF | Enable OpenGL renderer |
| `MLN_WITH_EGL` | OFF | Enable EGL renderer |
| `MLN_WITH_VULKAN` | OFF | Enable Vulkan renderer |
| `MLN_WITH_METAL` | OFF | Enable Metal renderer |
| `MLN_WITH_WEBGPU` | OFF | Enable WebGPU renderer |
| `MLN_WITH_PMTILES` | ON | Enable PMTiles support |
| `MLN_USE_RUST` | OFF | Use Rust components |
| `MLN_WITH_WERROR` | OFF | Treat warnings as errors |

### 12.2 Build Targets

- `mbgl-core` - Core static library
- `mbgl-glfw` - GLFW demo application
- `mbgl-offline` - Offline pack tool
- `mbgl-render` - Render tool
- Platform-specific targets (iOS framework, Android AAR, etc.)
- Test executables

### 12.3 Build Systems Used

| System | Purpose |
|--------|---------|
| **CMake** | Primary build system for C++ core |
| **Make** | Master Makefile coordinating builds |
| **Bazel** | Alternative build system (BUILD.bazel) |
| **Gradle** | Android builds |
| **Xcode** | iOS/macOS builds |
| **npm** | Development dependencies |
| **Mason** | C/C++ package manager for dependencies |

---

## 13. Design Patterns

### 13.1 Immutability Pattern

- `Immutable<T>` template for thread-safe sharing
- Style objects use immutable `Impl` classes
- Copy-on-write for mutations

```cpp
// Immutable wrapper (similar to shared_ptr<const T>)
template <typename T>
class Immutable {
    std::shared_ptr<const T> ptr;
};

// Usage in layers
class Layer {
    Immutable<Layer::Impl> impl;  // Immutable reference
};

// Mutation pattern
void CircleLayer::setCircleRadius(float radius) {
    auto newImpl = std::make_unique<CircleLayer::Impl>(*impl);  // Copy
    newImpl->circleRadius = radius;                              // Modify
    impl = Immutable<CircleLayer::Impl>(std::move(newImpl));    // Freeze
}
```

### 13.2 Actor Model

- Message-passing between threads
- `Actor<T>` wraps objects with mailbox
- Prevents shared mutable state

### 13.3 Observer Pattern

| Observer | File | Purpose |
|----------|------|---------|
| MapObserver | `include/mbgl/map/map_observer.hpp` | Map events |
| RendererObserver | `include/mbgl/renderer/renderer_observer.hpp` | Rendering events |
| TileObserver | `src/mbgl/tile/tile_observer.hpp` | Tile lifecycle |
| GlyphManagerObserver | `src/mbgl/text/glyph_manager_observer.hpp` | Glyph loading |
| ImageManagerObserver | `src/mbgl/renderer/image_manager_observer.hpp` | Image loading |
| RenderSourceObserver | `src/mbgl/renderer/render_source_observer.hpp` | Source events |
| ContextObserver | `include/mbgl/gfx/context_observer.hpp` | Graphics context events |

### 13.4 Factory Pattern

- Layer factories in `include/mbgl/layermanager/`
- `LayerFactory`, `BackgroundLayerFactory`, etc.
- Backend-specific object creation via `Backend::Create<T>()`

### 13.5 Strategy Pattern

- Graphics backend selection at compile time
- Pluggable render backends (OpenGL/Vulkan/Metal/WebGPU)
- File source strategies (HTTP, local, offline, PMTiles)

### 13.6 Builder Pattern

- `DrawableBuilder` for constructing Drawable objects
- SDK-specific builder implementations

### 13.7 PIMPL Idiom

- `Map::Impl`, `Renderer::Impl`, `Style::Impl`
- Separates public API from implementation
- Reduces compilation dependencies

---

## 14. Key Data Structures

### 14.1 Tile ID System

**File:** `include/mbgl/tile/tile_id.hpp`

```cpp
// Basic tile identification
struct CanonicalTileID {
    uint8_t z;     // Zoom level
    uint32_t x;    // X coordinate
    uint32_t y;    // Y coordinate
};

// Supports tile overscaling (rendering at different zoom than source)
struct OverscaledTileID {
    uint8_t overscaledZ;  // Zoom level for rendering
    uint8_t z;            // Source zoom level
    uint32_t x;           // X coordinate
    uint32_t y;           // Y coordinate
    uint32_t wrap;        // World wrap (for infinite horizontal scrolling)
};

// Includes world wrap information for rendering
struct UnwrappedTileID {
    uint8_t z;
    int32_t x;    // Can be negative for wrapped tiles
    uint32_t y;
};
```

### 14.2 TransformState

**File:** `src/mbgl/map/transform_state.hpp`

Camera and viewport state:
- Camera position (latitude, longitude)
- Zoom level
- Bearing (rotation)
- Pitch (tilt)
- Viewport dimensions
- Projection matrices (view, projection, model)

### 14.3 FeatureIndex

**File:** `src/mbgl/geometry/feature_index.hpp`

Maps screen coordinates to features for query operations:
- `queryRenderedFeatures()` - Features at screen point
- `querySourceFeatures()` - Features in source data

### 14.4 CollisionIndex

**File:** `src/mbgl/text/collision_index.hpp`

Symbol placement collision detection:
- Cross-tile symbol deduplication
- Label collision avoidance
- Icon collision avoidance

### 14.5 PlacementController

Controls symbol placement across the map:
- Manages placement state
- Coordinates with CrossTileSymbolIndex
- Handles fade transitions

---

## 15. Shader System

### 15.1 Shader Files

Located in `shaders/`:

| Layer | Vertex Shader | Fragment Shader |
|-------|---------------|-----------------|
| Background | `background.vertex.glsl` | `background.fragment.glsl` |
| Background Pattern | `background_pattern.vertex.glsl` | `background_pattern.fragment.glsl` |
| Circle | `circle.vertex.glsl` | `circle.fragment.glsl` |
| Fill | `fill.vertex.glsl` | `fill.fragment.glsl` |
| Fill Pattern | `fill_pattern.vertex.glsl` | `fill_pattern.fragment.glsl` |
| Fill Outline | `fill_outline.vertex.glsl` | `fill_outline.fragment.glsl` |
| Fill Outline Pattern | `fill_outline_pattern.vertex.glsl` | `fill_outline_pattern.fragment.glsl` |
| Fill Outline Triangulated | `fill_outline_triangulated.vertex.glsl` | `fill_outline_triangulated.fragment.glsl` |
| Fill Extrusion | `fill_extrusion.vertex.glsl` | `fill_extrusion.fragment.glsl` |
| Fill Extrusion Pattern | `fill_extrusion_pattern.vertex.glsl` | `fill_extrusion_pattern.fragment.glsl` |
| Line | `line.vertex.glsl` | `line.fragment.glsl` |
| Line Gradient | `line_gradient.vertex.glsl` | `line_gradient.fragment.glsl` |
| Line Pattern | `line_pattern.vertex.glsl` | `line_pattern.fragment.glsl` |
| Line SDF | `line_sdf.vertex.glsl` | `line_sdf.fragment.glsl` |
| Raster | `raster.vertex.glsl` | `raster.fragment.glsl` |
| Symbol Icon | `symbol_icon.vertex.glsl` | `symbol_icon.fragment.glsl` |
| Symbol SDF | `symbol_sdf.vertex.glsl` | `symbol_sdf.fragment.glsl` |
| Symbol Text and Icon | `symbol_text_and_icon.vertex.glsl` | `symbol_text_and_icon.fragment.glsl` |
| Heatmap | `heatmap.vertex.glsl` | `heatmap.fragment.glsl` |
| Heatmap Texture | `heatmap_texture.vertex.glsl` | `heatmap_texture.fragment.glsl` |
| Hillshade | `hillshade.vertex.glsl` | `hillshade.fragment.glsl` |
| Hillshade Prepare | `hillshade_prepare.vertex.glsl` | `hillshade_prepare.fragment.glsl` |
| Color Relief | `color_relief.vertex.glsl` | `color_relief.fragment.glsl` |
| Location Indicator | `location_indicator.vertex.glsl` | `location_indicator.fragment.glsl` |
| Location Indicator Textured | `location_indicator_textured.vertex.glsl` | `location_indicator_textured.fragment.glsl` |
| Custom Geometry | `custom_geometry.vertex.glsl` | `custom_geometry.fragment.glsl` |
| Collision Box | `collision_box.vertex.glsl` | `collision_box.fragment.glsl` |
| Collision Circle | `collision_circle.vertex.glsl` | `collision_circle.fragment.glsl` |
| Debug | `debug.vertex.glsl` | `debug.fragment.glsl` |
| Clipping Mask | `clipping_mask.vertex.glsl` | `clipping_mask.fragment.glsl` |

### 15.2 Shader Abstraction

**Shader Registry:** `include/mbgl/gfx/shader_registry.hpp`
- Manages shader compilation and lookup
- Supports runtime shader replacement

**Shader Program Base:** `include/mbgl/shaders/shader_program_base.hpp`
- Abstract base for all shader programs

### 15.3 Uniform Buffer Objects (UBOs)

Located in `include/mbgl/shaders/`:
- `background_layer_ubo.hpp`
- `circle_layer_ubo.hpp`
- `fill_layer_ubo.hpp`
- `fill_extrusion_layer_ubo.hpp`
- `line_layer_ubo.hpp`
- `symbol_layer_ubo.hpp`
- `raster_layer_ubo.hpp`
- `heatmap_layer_ubo.hpp`
- `hillshade_layer_ubo.hpp`

---

## 16. Text & Glyph System

### 16.1 GlyphManager

**File:** `src/mbgl/text/glyph_manager.hpp`

```cpp
class GlyphManager {
public:
    // Request glyphs for a font stack and range
    void getGlyphs(GlyphRequestor&, const FontStack&, const GlyphRange&);

    // Observer callbacks
    void setObserver(GlyphManagerObserver*);

    // Memory management
    void reduceMemoryUse();
};
```

### 16.2 Text Processing Pipeline

```
Text String
    │
    ▼
┌──────────────┐
│  Bidi        │  RTL/LTR text direction handling
│  Processing  │  (src/mbgl/text/bidi.hpp)
└──────────────┘
    │
    ▼
┌──────────────┐
│  HarfBuzz    │  Text shaping (src/mbgl/text/harfbuzz.hpp)
│  Shaping     │  - Glyph positioning
│              │  - Ligature handling
└──────────────┘
    │
    ▼
┌──────────────┐
│  Glyph       │  Glyph request to GlyphManager
│  Request     │  - Font stack
│              │  - Glyph range
└──────────────┘
    │
    ▼
┌──────────────┐
│  Glyph       │  PBF glyph parsing
│  PBF Parser  │  (src/mbgl/text/glyph_pbf.hpp)
└──────────────┘
    │
    ▼
┌──────────────┐
│  Glyph       │  Glyph atlas packing
│  Atlas       │  - DynamicTextureAtlas
└──────────────┘
    │
    ▼
┌──────────────┐
│  Symbol      │  Symbol layout and placement
│  Layout      │  (src/mbgl/layout/symbol_layout.hpp)
└──────────────┘
    │
    ▼
┌──────────────┐
│  Collision   │  Collision detection
│  Detection   │  (src/mbgl/text/collision_index.hpp)
└──────────────┘
    │
    ▼
┌──────────────┐
│  Render      │  GPU rendering via SDF shaders
│  (SDF)       │  - symbol_sdf.*.glsl
└──────────────┘
```

### 16.3 Key Text Components

| Component | File | Purpose |
|-----------|------|---------|
| GlyphManager | `src/mbgl/text/glyph_manager.hpp` | Glyph loading and caching |
| GlyphManagerObserver | `src/mbgl/text/glyph_manager_observer.hpp` | Glyph load callbacks |
| HBShape | `src/mbgl/text/harfbuzz.hpp` | HarfBuzz text shaping |
| Shaping | `src/mbgl/text/shaping.hpp` | Text shaping results |
| Quads | `src/mbgl/text/quads.hpp` | Symbol quad generation |
| CollisionIndex | `src/mbgl/text/collision_index.hpp` | Collision detection |
| CrossTileSymbolIndex | `src/mbgl/text/cross_tile_symbol_index.hpp` | Cross-tile deduplication |
| Placement | `src/mbgl/text/placement.hpp` | Symbol placement controller |
| Bidi | `src/mbgl/text/bidi.hpp` | Bidirectional text support |
| FreeType | `src/mbgl/text/freetype.hpp` | Font rasterization |

---

## 17. Layout System

### 17.1 Layout Types

Located in `src/mbgl/layout/`:

| Layout | File | Purpose |
|--------|------|---------|
| Layout | `layout.hpp` | Base layout template |
| SymbolLayout | `symbol_layout.hpp` | Text and icon symbol layout |
| SymbolInstance | `symbol_instance.hpp` | Individual symbol instance |
| SymbolFeature | `symbol_feature.hpp` | Symbol feature processing |
| SymbolProjection | `symbol_projection.hpp` | Symbol projection math |
| CircleLayout | `circle_layout.hpp` | Circle feature layout |
| PatternLayout | `pattern_layout.hpp` | Pattern feature layout |
| ClipLines | `clip_lines.hpp` | Line clipping utilities |
| MergeLines | `merge_lines.hpp` | Line merging utilities |

### 17.2 Symbol Layout Process

```
Vector Tile Features
    │
    ▼
┌──────────────────┐
│  SymbolFeature   │  Process feature geometry
│  Processing      │  - Extract text/icon properties
└──────────────────┘
    │
    ▼
┌──────────────────┐
│  SymbolLayout    │  Create layout for layer
│  Creation        │  - Request glyphs
│                  │  - Request images
└──────────────────┘
    │
    ▼
┌──────────────────┐
│  SymbolInstance  │  Create instances for each label
│  Creation        │  - Position calculation
│                  │  - Anchor points
└──────────────────┘
    │
    ▼
┌──────────────────┐
│  Collision       │  Check for overlaps
│  Detection       │  - Tile boundaries
│                  │  - Other symbols
└──────────────────┘
    │
    ▼
┌──────────────────┐
│  Placement       │  Final placement decision
│  Decision        │  - Place or hide
│                  │  - Fade state
└──────────────────┘
```

---

## 18. Storage & Caching

### 18.1 FileSource Types

Located in `src/mbgl/storage/`:

| Source | File | Purpose |
|--------|------|---------|
| HTTPFileSource | `http_file_source.hpp` | HTTP/HTTPS network requests |
| LocalFileSource | `local_file_source.hpp` | Local file access |
| AssetFileSource | `asset_file_source.hpp` | Platform asset access |
| MBTilesFileSource | `mbtiles_file_source.hpp` | MBTiles database access |
| PMTilesFileSource | `pmtiles_file_source.hpp` | PMTiles archive access |
| MainResourceLoader | `main_resource_loader.hpp` | Resource loading coordination |

### 18.2 Resource Types

**File:** `src/mbgl/storage/resource.cpp`

```cpp
enum class Kind : uint8_t {
    Style,           // Style JSON
    SpriteJSON,      // Sprite metadata
    SpriteImage,     // Sprite image
    Glyphs,          // Font glyphs
    Source,          // Tile source data
    Tile,            // Map tile
    Custom           // Custom resource
};
```

### 18.3 Response Structure

**File:** `src/mbgl/storage/response.cpp`

```cpp
class Response {
    Error error;              // Error information
    std::string data;         // Response body
    Timestamp modified;       // Last-Modified header
    Timestamp expires;        // Expires header
    std::string etag;         // ETag header
    bool mustRevalidate;      // Cache revalidation flag
};
```

---

## Appendix A: File Count Summary

| Category | Headers | Sources | Total |
|----------|---------|---------|-------|
| Tile System | 12 | 28 | 40 |
| Renderer | 25 | 36 | 61 |
| Render Layers | 13 | 13 | 26 |
| Layer Tweakers | 13 | 13 | 26 |
| Buckets | 9 | 9 | 18 |
| Graphics (gfx) | 37 | - | 37 |
| OpenGL Backend | 10 | - | 10 |
| Vulkan Backend | 20 | - | 20 |
| Metal Backend | 19 | - | 19 |
| Text/Glyphs | 5 | 29 | 34 |
| Layout | 6 | 8 | 14 |
| Storage | 1 | 11 | 12 |
| Actor Framework | 9 | - | 9 |
| Style System | 25 | - | 25 |
| Shaders (GLSL) | - | - | 68 |

**Total Core Files:** ~450+ files

---

## Appendix B: Key File Paths Reference

| Component | Header Path | Source Path |
|-----------|-------------|-------------|
| Map | `include/mbgl/map/map.hpp` | `src/mbgl/map/map_impl.cpp` |
| Renderer | `include/mbgl/renderer/renderer.hpp` | `src/mbgl/renderer/renderer_impl.cpp` |
| RenderOrchestrator | - | `src/mbgl/renderer/render_orchestrator.hpp` |
| Tile | - | `src/mbgl/tile/tile.hpp` |
| GeometryTile | - | `src/mbgl/tile/geometry_tile.hpp` |
| GeometryTileWorker | - | `src/mbgl/tile/geometry_tile_worker.hpp` |
| TilePyramid | - | `src/mbgl/renderer/tile_pyramid.hpp` |
| Bucket | - | `src/mbgl/renderer/bucket.hpp` |
| Drawable | `include/mbgl/gfx/drawable.hpp` | `src/mbgl/gfx/drawable.cpp` |
| Context | `include/mbgl/gfx/context.hpp` | `src/mbgl/gfx/context.cpp` |
| Backend | `include/mbgl/gfx/backend.hpp` | `src/mbgl/gfx/backend.cpp` |
| Scheduler | `include/mbgl/actor/scheduler.hpp` | `src/mbgl/actor/scheduler.cpp` |
| Style | `include/mbgl/style/style.hpp` | `src/mbgl/style/style.cpp` |
| GlyphManager | - | `src/mbgl/text/glyph_manager.hpp` |
| ImageManager | - | `src/mbgl/renderer/image_manager.hpp` |

---

## Appendix C: Render Pass Types

```cpp
enum class RenderPass : uint8_t {
    Opaque = 1 << 0,      // Opaque drawables (no blending)
    Transparent = 1 << 1, // Transparent drawables (blending)
    NumRenderPasses = 2
};
```

---

## Appendix D: Map Debug Options

```cpp
enum class MapDebugOptions {
    NoDebug = 0,          // No debug
    TileBorders = 1,      // Show tile boundaries
    ParseStatus = 2,      // Show tile parse status
    Timestamps = 4,       // Show tile timestamps
    Collision = 8,        // Show symbol collision boxes
    Overdraw = 16,        // Show overdraw visualization
    StencilClip = 32,     // Show stencil clipping
    DepthBuffer = 64,     // Show depth buffer
    RenderBatch = 128     // Show render batches
};
```

---

*This document provides a comprehensive analysis of the MapLibre Native architecture. For implementation details, refer to the source files listed throughout this document.*
