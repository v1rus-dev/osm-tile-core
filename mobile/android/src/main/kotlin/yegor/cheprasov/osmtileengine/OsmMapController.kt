package yegor.cheprasov.osmtileengine

import android.animation.Animator
import android.animation.AnimatorListenerAdapter
import android.animation.ValueAnimator
import android.view.Surface
import android.view.animation.DecelerateInterpolator
import androidx.annotation.Keep
import java.lang.ref.WeakReference
import kotlin.math.PI
import kotlin.math.atan
import kotlin.math.floor
import kotlin.math.ln
import kotlin.math.pow
import kotlin.math.sin
import kotlin.math.sinh

internal const val DEFAULT_CACHE_DIR = "tile-cache"
internal const val TILE_SIZE_PX = 256.0
internal const val MIN_LAT = -85.05112878
internal const val MAX_LAT = 85.05112878
internal const val MIN_ZOOM = 0.0
internal const val MAX_ZOOM = 30.0
internal const val MAX_PITCH = 85.0

@Keep
data class OsmCoordinate(
    val lat: Double,
    val lon: Double,
)

@Keep
data class OsmBounds(
    val south: Double,
    val west: Double,
    val north: Double,
    val east: Double,
)

@Keep
data class OsmCamera(
    val centerLat: Double = 0.0,
    val centerLon: Double = 0.0,
    val zoom: Double = 0.0,
    val bearing: Double = 0.0,
    val pitch: Double = 0.0,
)

@Keep
enum class CameraChangeReason {
    GESTURE,
    PROGRAMMATIC,
    RESTORE,
}

@Keep
fun interface OnCameraChangedListener {
    fun onCameraChanged(camera: OsmCamera, reason: CameraChangeReason)
}

private data class TileLayerState(
    val layerId: String,
    val urlTemplate: String?,
    val zIndex: Int,
    val opacity: Float,
    val visible: Boolean,
)

private data class PendingFitBoundsRequest(
    val bounds: OsmBounds,
    val paddingDp: Float,
    val durationMs: Long,
)

internal object OsmNativeLibrary {
    @Volatile private var loaded = false

    fun ensureLoaded() {
        if (loaded) return
        synchronized(this) {
            if (loaded) return
            System.loadLibrary("osm_tile_engine")
            loaded = true
        }
    }
}

internal fun wrapLongitude(value: Double): Double {
    var lon = value
    while (lon < -180.0) lon += 360.0
    while (lon > 180.0) lon -= 360.0
    return lon
}

internal fun wrapWorldCoordinate(value: Double, worldSize: Double): Double {
    if (worldSize <= 0.0) return 0.0
    var wrapped = value % worldSize
    if (wrapped < 0.0) {
        wrapped += worldSize
    }
    return wrapped
}

internal fun worldSizePx(zoom: Double): Double = TILE_SIZE_PX * 2.0.pow(zoom)

internal fun OsmCoordinate.normalized(): OsmCoordinate {
    require(lat.isFinite()) { "Coordinate latitude must be finite" }
    require(lon.isFinite()) { "Coordinate longitude must be finite" }
    require(lat in -90.0..90.0) { "Coordinate latitude must be in -90..90" }
    return copy(
        lat = lat.coerceIn(MIN_LAT, MAX_LAT),
        lon = wrapLongitude(lon),
    )
}

internal fun OsmBounds.normalized(): OsmBounds {
    require(south.isFinite() && north.isFinite()) { "Bounds latitude must be finite" }
    require(west.isFinite() && east.isFinite()) { "Bounds longitude must be finite" }
    require(south in -90.0..90.0) { "Bounds south must be in -90..90" }
    require(north in -90.0..90.0) { "Bounds north must be in -90..90" }
    require(south <= north) { "Bounds south must be less than or equal to north" }

    return copy(
        south = south.coerceIn(MIN_LAT, MAX_LAT),
        north = north.coerceIn(MIN_LAT, MAX_LAT),
        west = wrapLongitude(west),
        east = wrapLongitude(east),
    )
}

internal fun OsmCamera.normalized(): OsmCamera =
    copy(
        centerLat = centerLat.coerceIn(MIN_LAT, MAX_LAT),
        centerLon = wrapLongitude(centerLon),
        zoom = zoom.coerceIn(MIN_ZOOM, MAX_ZOOM),
        pitch = pitch.coerceIn(0.0, MAX_PITCH),
    )

internal fun longitudeToNormalizedX(value: Double): Double = (wrapLongitude(value) + 180.0) / 360.0

internal fun latitudeToNormalizedY(value: Double): Double {
    val clamped = value.coerceIn(MIN_LAT, MAX_LAT)
    val sinLat = sin(clamped * PI / 180.0)
    return 0.5 - ln((1.0 + sinLat) / (1.0 - sinLat)) / (4.0 * PI)
}

internal fun normalizedXToLongitude(value: Double): Double {
    val wrapped = value - floor(value)
    return wrapLongitude(wrapped * 360.0 - 180.0)
}

internal fun normalizedYToLatitude(value: Double): Double {
    val clamped = value.coerceIn(0.0, 1.0)
    val mercator = PI * (1.0 - 2.0 * clamped)
    return atan(sinh(mercator)) * 180.0 / PI
}

@Keep
class OsmMapController private constructor(
    private var engine: OsmTileEngine,
    private val ownsEngine: Boolean,
    private var defaultTileUrlTemplate: String?,
    private var cacheDir: String?,
) : AutoCloseable {
    private var cameraAnimator: ValueAnimator? = null
    private var pendingFitBounds: PendingFitBoundsRequest? = null
    private var nativeHandle = 0L
    private var camera = OsmCamera().normalized()
    private var surfaceReady = false
    private var surfaceWidthPx = 0
    private var surfaceHeightPx = 0
    private var surfaceDensity = 1f
    private var attachedSurface: Surface? = null
    private var attachedView: WeakReference<OsmMapView>? = null
    private var destroyed = false

    private val tileLayers = LinkedHashMap<String, TileLayerState>()
    private val internalObservers = LinkedHashMap<Any, (OsmCamera, CameraChangeReason) -> Unit>()
    private var cameraChangedListener: OnCameraChangedListener? = null

    init {
        OsmNativeLibrary.ensureLoaded()
        nativeHandle = nativeCreateRendererFromEngine(engine.uniffiCloneHandle())
        check(nativeHandle != 0L) { "Failed to create native renderer for OsmMapController" }
        pushCameraToNative()
    }

    constructor(tileUrlTemplate: String, cacheDir: String) : this(
        engine = OsmTileEngine(tileUrlTemplate, cacheDir),
        ownsEngine = true,
        defaultTileUrlTemplate = tileUrlTemplate,
        cacheDir = cacheDir,
    )

    constructor(engine: OsmTileEngine) : this(
        engine = engine,
        ownsEngine = false,
        defaultTileUrlTemplate = null,
        cacheDir = null,
    )

    fun getCamera(): OsmCamera = camera

    fun getEngine(): OsmTileEngine = engine

    fun setCamera(
        camera: OsmCamera,
        reason: CameraChangeReason = CameraChangeReason.PROGRAMMATIC,
    ) {
        ensureNotDestroyed()
        applyCamera(camera, reason, cancelAnimation = true)
    }

    fun setOnCameraChangedListener(listener: OnCameraChangedListener?) {
        ensureNotDestroyed()
        cameraChangedListener = listener
    }

    fun moveCamera(camera: OsmCamera) {
        setCamera(camera, CameraChangeReason.PROGRAMMATIC)
    }

    fun animateCamera(
        camera: OsmCamera,
        durationMs: Long = 300L,
    ) {
        ensureNotDestroyed()

        if (durationMs <= 0L) {
            moveCamera(camera)
            return
        }

        val startCamera = this.camera
        val endCamera = camera.normalized()
        if (startCamera == endCamera) {
            applyCamera(endCamera, CameraChangeReason.PROGRAMMATIC, cancelAnimation = true)
            return
        }

        cameraAnimator?.cancel()
        val animator = ValueAnimator.ofFloat(0f, 1f).apply {
            duration = durationMs
            interpolator = DecelerateInterpolator()
            addUpdateListener { valueAnimator ->
                val fraction = valueAnimator.animatedValue as Float
                val interpolated = interpolateCamera(startCamera, endCamera, fraction)
                applyCamera(
                    interpolated,
                    CameraChangeReason.PROGRAMMATIC,
                    cancelAnimation = false,
                )
            }
            addListener(object : AnimatorListenerAdapter() {
                override fun onAnimationEnd(animation: Animator) {
                    if (cameraAnimator === animation) {
                        cameraAnimator = null
                    }
                }

                override fun onAnimationCancel(animation: Animator) {
                    if (cameraAnimator === animation) {
                        cameraAnimator = null
                    }
                }
            })
        }

        cameraAnimator = animator
        animator.start()
    }

    fun fitBounds(
        bounds: OsmBounds,
        paddingDp: Float = 0f,
    ) {
        fitBounds(bounds, paddingDp, durationMs = 0L)
    }

    fun fitBounds(
        bounds: OsmBounds,
        paddingDp: Float = 0f,
        durationMs: Long,
    ) {
        ensureNotDestroyed()
        val normalizedBounds = bounds.normalized()
        val nonNegativePadding = paddingDp.coerceAtLeast(0f)

        if (surfaceWidthPx <= 0 || surfaceHeightPx <= 0) {
            pendingFitBounds = PendingFitBoundsRequest(
                bounds = normalizedBounds,
                paddingDp = nonNegativePadding,
                durationMs = durationMs,
            )
            return
        }

        val fittedCamera = computeCameraForBounds(normalizedBounds, nonNegativePadding)
        pendingFitBounds = null
        if (durationMs > 0L) {
            animateCamera(fittedCamera, durationMs)
        } else {
            moveCamera(fittedCamera)
        }
    }

    fun fitBounds(
        coordinates: Iterable<OsmCoordinate>,
        paddingDp: Float = 0f,
    ) {
        fitBounds(boundsFromCoordinates(coordinates), paddingDp, durationMs = 0L)
    }

    fun fitBounds(
        coordinates: Iterable<OsmCoordinate>,
        paddingDp: Float = 0f,
        durationMs: Long,
    ) {
        fitBounds(boundsFromCoordinates(coordinates), paddingDp, durationMs)
    }

    fun fitBounds(
        markers: List<MobileMarker>,
        paddingDp: Float = 0f,
    ) {
        fitBounds(boundsFromCoordinates(markers.asSequence().map { OsmCoordinate(it.lat, it.lon) }.asIterable()), paddingDp, durationMs = 0L)
    }

    fun fitBounds(
        markers: List<MobileMarker>,
        paddingDp: Float = 0f,
        durationMs: Long,
    ) {
        fitBounds(boundsFromCoordinates(markers.asSequence().map { OsmCoordinate(it.lat, it.lon) }.asIterable()), paddingDp, durationMs)
    }

    fun stopAnimation() {
        ensureNotDestroyed()
        pendingFitBounds = null
        cameraAnimator?.cancel()
        cameraAnimator = null
    }

    fun addTileLayer(
        layerId: String,
        urlTemplate: String? = null,
        zIndex: Int = 0,
        opacity: Float = 1f,
    ) {
        ensureNotDestroyed()
        val layerState = TileLayerState(
            layerId = layerId,
            urlTemplate = urlTemplate,
            zIndex = zIndex,
            opacity = opacity.coerceIn(0f, 1f),
            visible = tileLayers[layerId]?.visible ?: true,
        )
        tileLayers[layerId] = layerState
        if (nativeHandle == 0L) return
        nativeAddTileLayer(
            nativeHandle,
            layerId,
            resolveUrlTemplate(layerState),
            zIndex,
            layerState.opacity,
        )
        if (!layerState.visible) {
            nativeSetLayerVisible(nativeHandle, layerId, false)
        }
    }

    fun removeLayer(layerId: String) {
        ensureNotDestroyed()
        tileLayers.remove(layerId)
        if (nativeHandle != 0L) {
            nativeRemoveLayer(nativeHandle, layerId)
        }
    }

    fun setLayerVisible(layerId: String, visible: Boolean) {
        ensureNotDestroyed()
        tileLayers[layerId]?.let { state ->
            tileLayers[layerId] = state.copy(visible = visible)
        }
        if (nativeHandle != 0L) {
            nativeSetLayerVisible(nativeHandle, layerId, visible)
        }
    }

    fun setLayerOpacity(layerId: String, opacity: Float) {
        ensureNotDestroyed()
        val clampedOpacity = opacity.coerceIn(0f, 1f)
        tileLayers[layerId]?.let { state ->
            tileLayers[layerId] = state.copy(opacity = clampedOpacity)
        }
        if (nativeHandle != 0L) {
            nativeSetLayerOpacity(nativeHandle, layerId, clampedOpacity)
        }
    }

    override fun close() {
        destroy()
    }

    fun destroy() {
        if (destroyed) return
        cameraAnimator?.cancel()
        cameraAnimator = null
        pendingFitBounds = null

        if (surfaceReady && nativeHandle != 0L) {
            nativeSurfaceDestroyed(nativeHandle)
        }
        if (nativeHandle != 0L) {
            nativeDestroyRenderer(nativeHandle)
            nativeHandle = 0L
        }

        destroyed = true
        attachedSurface = null
        surfaceReady = false
        cameraChangedListener = null
        internalObservers.clear()
        attachedView?.get()?.onControllerDestroyed(this)
        attachedView = null
        if (ownsEngine) {
            engine.destroy()
        }
    }

    internal fun attachToView(view: OsmMapView) {
        ensureNotDestroyed()
        val existing = attachedView?.get()
        require(existing == null || existing === view) {
            "OsmMapController is already attached to another OsmMapView"
        }
        attachedView = WeakReference(view)
    }

    internal fun detachFromView(view: OsmMapView) {
        if (attachedView?.get() === view) {
            attachedView = null
        }
    }

    internal fun addInternalCameraObserver(
        owner: Any,
        observer: (OsmCamera, CameraChangeReason) -> Unit,
    ) {
        ensureNotDestroyed()
        internalObservers[owner] = observer
    }

    internal fun removeInternalCameraObserver(owner: Any) {
        internalObservers.remove(owner)
    }

    internal fun onSurfaceCreated(surface: Surface) {
        ensureNotDestroyed()
        surfaceReady = true
        attachedSurface = surface
        if (nativeHandle != 0L && surface.isValid) {
            nativeSurfaceCreated(nativeHandle, surface)
            pushResizeIfReady()
            pushCameraToNative()
        }
        dispatchCameraChanged(camera, CameraChangeReason.RESTORE)
    }

    internal fun onSurfaceChanged(widthPx: Int, heightPx: Int, density: Float) {
        ensureNotDestroyed()
        surfaceWidthPx = widthPx
        surfaceHeightPx = heightPx
        surfaceDensity = density
        pushResizeIfReady()
        applyPendingFitBoundsIfPossible()
    }

    internal fun onSurfaceDestroyed() {
        if (destroyed) return
        surfaceReady = false
        attachedSurface = null
        if (nativeHandle != 0L) {
            nativeSurfaceDestroyed(nativeHandle)
        }
    }

    internal fun updateRendererConfig(tileUrlTemplate: String, cacheDir: String) {
        ensureNotDestroyed()
        check(ownsEngine) {
            "updateRendererConfig is only supported for controllers that own their OsmTileEngine"
        }
        if (defaultTileUrlTemplate == tileUrlTemplate && this.cacheDir == cacheDir) return
        defaultTileUrlTemplate = tileUrlTemplate
        this.cacheDir = cacheDir
        recreateNativeRenderer()
    }

    private fun recreateNativeRenderer() {
        cameraAnimator?.cancel()
        cameraAnimator = null
        if (surfaceReady && nativeHandle != 0L) {
            nativeSurfaceDestroyed(nativeHandle)
        }
        if (nativeHandle != 0L) {
            nativeDestroyRenderer(nativeHandle)
        }

        if (ownsEngine) {
            engine.destroy()
            engine = OsmTileEngine(
                requireNotNull(defaultTileUrlTemplate),
                requireNotNull(cacheDir),
            )
        }
        nativeHandle = nativeCreateRendererFromEngine(engine.uniffiCloneHandle())
        check(nativeHandle != 0L) { "Failed to recreate native renderer for OsmMapController" }
        replayLayerState()

        val surface = attachedSurface
        if (surfaceReady && surface != null && surface.isValid && nativeHandle != 0L) {
            nativeSurfaceCreated(nativeHandle, surface)
            pushResizeIfReady()
        }
        pushCameraToNative()
        dispatchCameraChanged(camera, CameraChangeReason.RESTORE)
        applyPendingFitBoundsIfPossible()
    }

    private fun replayLayerState() {
        if (nativeHandle == 0L) return

        for (layerState in tileLayers.values) {
            nativeAddTileLayer(
                nativeHandle,
                layerState.layerId,
                resolveUrlTemplate(layerState),
                layerState.zIndex,
                layerState.opacity,
            )
            if (!layerState.visible) {
                nativeSetLayerVisible(nativeHandle, layerState.layerId, false)
            }
        }
    }

    private fun resolveUrlTemplate(layerState: TileLayerState): String =
        layerState.urlTemplate ?: requireNotNull(defaultTileUrlTemplate) {
            "A urlTemplate is required for extra tile layers when OsmMapController is created from an external OsmTileEngine"
        }

    private fun pushResizeIfReady() {
        if (!surfaceReady || nativeHandle == 0L || surfaceWidthPx <= 0 || surfaceHeightPx <= 0) {
            return
        }
        nativeSurfaceChanged(nativeHandle, surfaceWidthPx, surfaceHeightPx, surfaceDensity)
    }

    private fun pushCameraToNative() {
        if (nativeHandle == 0L) return
        nativeSetCamera(
            nativeHandle,
            camera.centerLat,
            camera.centerLon,
            camera.zoom,
            camera.bearing,
            camera.pitch,
        )
    }

    private fun applyCamera(
        camera: OsmCamera,
        reason: CameraChangeReason,
        cancelAnimation: Boolean,
    ) {
        if (cancelAnimation) {
            cameraAnimator?.cancel()
            cameraAnimator = null
        }
        pendingFitBounds = null
        val normalized = camera.normalized()
        this.camera = normalized
        pushCameraToNative()
        dispatchCameraChanged(normalized, reason)
    }

    private fun computeCameraForBounds(bounds: OsmBounds, paddingDp: Float): OsmCamera {
        val paddingPx = paddingDp * surfaceDensity
        val usableWidthPx = (surfaceWidthPx - paddingPx * 2f).coerceAtLeast(1f).toDouble()
        val usableHeightPx = (surfaceHeightPx - paddingPx * 2f).coerceAtLeast(1f).toDouble()

        val westX = longitudeToNormalizedX(bounds.west)
        val eastX = longitudeToNormalizedX(bounds.east)
        val spanX = if (bounds.west > bounds.east) {
            (eastX + 1.0) - westX
        } else {
            eastX - westX
        }.coerceAtLeast(0.0)
        val centerX = westX + spanX / 2.0

        val northY = latitudeToNormalizedY(bounds.north)
        val southY = latitudeToNormalizedY(bounds.south)
        val spanY = (southY - northY).coerceAtLeast(0.0)
        val centerY = northY + spanY / 2.0

        val zoomX = zoomToFitSpan(spanX, usableWidthPx)
        val zoomY = zoomToFitSpan(spanY, usableHeightPx)
        val targetZoom = minOf(zoomX, zoomY).coerceIn(MIN_ZOOM, MAX_ZOOM)

        return OsmCamera(
            centerLat = normalizedYToLatitude(centerY),
            centerLon = normalizedXToLongitude(centerX),
            zoom = targetZoom,
            bearing = camera.bearing,
            pitch = camera.pitch,
        ).normalized()
    }

    private fun zoomToFitSpan(span: Double, usableSizePx: Double): Double {
        if (span <= 0.0) {
            return MAX_ZOOM
        }
        val scale = usableSizePx / (TILE_SIZE_PX * span)
        if (!scale.isFinite() || scale <= 0.0) {
            return MIN_ZOOM
        }
        return kotlin.math.log2(scale)
    }

    private fun boundsFromCoordinates(coordinates: Iterable<OsmCoordinate>): OsmBounds {
        val normalizedCoordinates = coordinates.map { it.normalized() }.toList()
        require(normalizedCoordinates.isNotEmpty()) { "At least one coordinate is required to fit bounds" }

        val south = normalizedCoordinates.minOf { it.lat }
        val north = normalizedCoordinates.maxOf { it.lat }
        val normalizedLongitudes = normalizedCoordinates
            .map { longitudeToNormalizedDegrees(it.lon) }
            .sorted()

        if (normalizedLongitudes.size == 1) {
            val lon = normalizedCoordinates.first().lon
            return OsmBounds(
                south = south,
                west = lon,
                north = north,
                east = lon,
            )
        }

        var largestGap = -1.0
        var largestGapIndex = 0
        for (index in normalizedLongitudes.indices) {
            val current = normalizedLongitudes[index]
            val next = if (index == normalizedLongitudes.lastIndex) {
                normalizedLongitudes.first() + 360.0
            } else {
                normalizedLongitudes[index + 1]
            }
            val gap = next - current
            if (gap > largestGap) {
                largestGap = gap
                largestGapIndex = index
            }
        }

        val westNormalized =
            normalizedLongitudes[(largestGapIndex + 1) % normalizedLongitudes.size]
        val eastNormalized = normalizedLongitudes[largestGapIndex]

        return OsmBounds(
            south = south,
            west = normalizedDegreesToLongitude(westNormalized),
            north = north,
            east = normalizedDegreesToLongitude(eastNormalized),
        )
    }

    private fun interpolateCamera(
        start: OsmCamera,
        end: OsmCamera,
        fraction: Float,
    ): OsmCamera {
        val t = fraction.toDouble().coerceIn(0.0, 1.0)
        val startX = longitudeToNormalizedX(start.centerLon)
        val endX = longitudeToNormalizedX(end.centerLon)
        var deltaX = endX - startX
        if (deltaX > 0.5) deltaX -= 1.0
        if (deltaX < -0.5) deltaX += 1.0

        val startY = latitudeToNormalizedY(start.centerLat)
        val endY = latitudeToNormalizedY(end.centerLat)

        return OsmCamera(
            centerLat = normalizedYToLatitude(lerp(startY, endY, t)),
            centerLon = normalizedXToLongitude(startX + deltaX * t),
            zoom = lerp(start.zoom, end.zoom, t),
            bearing = lerpAngleDegrees(start.bearing, end.bearing, t),
            pitch = lerp(start.pitch, end.pitch, t),
        )
    }

    private fun lerp(start: Double, end: Double, t: Double): Double = start + (end - start) * t

    private fun lerpAngleDegrees(start: Double, end: Double, t: Double): Double {
        var delta = (end - start) % 360.0
        if (delta > 180.0) delta -= 360.0
        if (delta < -180.0) delta += 360.0
        return start + delta * t
    }

    private fun applyPendingFitBoundsIfPossible() {
        val request = pendingFitBounds ?: return
        if (surfaceWidthPx <= 0 || surfaceHeightPx <= 0) {
            return
        }
        pendingFitBounds = null
        fitBounds(request.bounds, request.paddingDp, request.durationMs)
    }

    private fun longitudeToNormalizedDegrees(lon: Double): Double =
        (wrapLongitude(lon) + 360.0) % 360.0

    private fun normalizedDegreesToLongitude(value: Double): Double {
        val wrapped = value % 360.0
        return wrapLongitude(if (wrapped < 0.0) wrapped + 360.0 else wrapped)
    }

    private fun dispatchCameraChanged(camera: OsmCamera, reason: CameraChangeReason) {
        val externalListener = cameraChangedListener
        val observers = internalObservers.values.toList()
        externalListener?.onCameraChanged(camera, reason)
        for (observer in observers) {
            observer(camera, reason)
        }
    }

    private fun ensureNotDestroyed() {
        check(!destroyed) { "OsmMapController has already been destroyed" }
    }

    private companion object {
        @JvmStatic
        private external fun nativeCreateRendererFromEngine(engineHandle: Long): Long

        @JvmStatic
        private external fun nativeSurfaceCreated(ptr: Long, surface: Surface)

        @JvmStatic
        private external fun nativeSurfaceChanged(
            ptr: Long,
            widthPx: Int,
            heightPx: Int,
            density: Float,
        )

        @JvmStatic
        private external fun nativeSetCamera(
            ptr: Long,
            centerLat: Double,
            centerLon: Double,
            zoom: Double,
            bearing: Double,
            pitch: Double,
        )

        @JvmStatic
        private external fun nativeSurfaceDestroyed(ptr: Long)

        @JvmStatic
        private external fun nativeDestroyRenderer(ptr: Long)

        @JvmStatic
        private external fun nativeAddTileLayer(
            ptr: Long,
            layerId: String,
            urlTemplate: String,
            zIndex: Int,
            opacity: Float,
        )

        @JvmStatic
        private external fun nativeRemoveLayer(ptr: Long, layerId: String)

        @JvmStatic
        private external fun nativeSetLayerVisible(ptr: Long, layerId: String, visible: Boolean)

        @JvmStatic
        private external fun nativeSetLayerOpacity(ptr: Long, layerId: String, opacity: Float)
    }
}
