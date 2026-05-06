package yegor.cheprasov.osmtileengine

import android.content.Context
import android.util.AttributeSet
import android.view.MotionEvent
import android.view.ScaleGestureDetector
import android.view.Surface
import android.view.SurfaceHolder
import android.view.SurfaceView
import androidx.annotation.Keep
import kotlin.math.ln
import kotlin.math.pow

@Keep
class OsmMapView @JvmOverloads constructor(
    context: Context,
    attrs: AttributeSet? = null,
    defStyleAttr: Int = 0,
) : SurfaceView(context, attrs, defStyleAttr), SurfaceHolder.Callback {
    private var nativeHandle: Long = 0L
    private var surfaceReady = false
    private var surfaceWidth = 0
    private var surfaceHeight = 0
    private var lastTouchX = 0f
    private var lastTouchY = 0f
    private var dragging = false

    private var tileUrlTemplate: String = DEFAULT_TILE_URL_TEMPLATE
    private var cacheDir: String = context.filesDir.resolve(DEFAULT_CACHE_DIR).absolutePath
    private var centerLat = 0.0
    private var centerLon = 0.0
    private var zoom = 0.0
    private var bearing = 0.0
    private var pitch = 0.0

    private val scaleDetector = ScaleGestureDetector(
        context,
        object : ScaleGestureDetector.SimpleOnScaleGestureListener() {
            override fun onScale(detector: ScaleGestureDetector): Boolean {
                val zoomDelta = ln(detector.scaleFactor.toDouble()) / ln(2.0)
                setCamera(
                    centerLat = centerLat,
                    centerLon = centerLon,
                    zoom = (zoom + zoomDelta).coerceIn(MIN_ZOOM, MAX_ZOOM),
                    bearing = bearing,
                    pitch = pitch,
                )
                return true
            }
        },
    )

    init {
        holder.addCallback(this)
        isFocusable = true
        isClickable = true
    }

    fun setTileUrlTemplate(template: String) {
        if (tileUrlTemplate == template) return
        tileUrlTemplate = template
        recreateRenderer()
    }

    fun setCacheDir(path: String) {
        if (cacheDir == path) return
        cacheDir = path
        recreateRenderer()
    }

    fun setCamera(
        centerLat: Double,
        centerLon: Double,
        zoom: Double,
        bearing: Double = 0.0,
        pitch: Double = 0.0,
    ) {
        this.centerLat = centerLat.coerceIn(MIN_LAT, MAX_LAT)
        this.centerLon = wrapLongitude(centerLon)
        this.zoom = zoom.coerceIn(MIN_ZOOM, MAX_ZOOM)
        this.bearing = bearing
        this.pitch = pitch.coerceIn(0.0, MAX_PITCH)
        nativeSetCamera(
            ensureRenderer(),
            this.centerLat,
            this.centerLon,
            this.zoom,
            this.bearing,
            this.pitch,
        )
    }

    fun addTileLayer(
        layerId: String,
        urlTemplate: String = tileUrlTemplate,
        zIndex: Int = 0,
        opacity: Float = 1f,
    ) {
        nativeAddTileLayer(ensureRenderer(), layerId, urlTemplate, zIndex, opacity.coerceIn(0f, 1f))
    }

    fun removeLayer(layerId: String) {
        nativeRemoveLayer(ensureRenderer(), layerId)
    }

    fun setLayerVisible(layerId: String, visible: Boolean) {
        nativeSetLayerVisible(ensureRenderer(), layerId, visible)
    }

    fun setLayerOpacity(layerId: String, opacity: Float) {
        nativeSetLayerOpacity(ensureRenderer(), layerId, opacity.coerceIn(0f, 1f))
    }

    override fun surfaceCreated(holder: SurfaceHolder) {
        surfaceReady = true
        val handle = ensureRenderer()
        nativeAttachSurface(handle, holder.surface)
        pushResizeIfReady()
        pushCamera()
    }

    override fun surfaceChanged(holder: SurfaceHolder, format: Int, width: Int, height: Int) {
        surfaceWidth = width
        surfaceHeight = height
        pushResizeIfReady()
    }

    override fun surfaceDestroyed(holder: SurfaceHolder) {
        surfaceReady = false
        if (nativeHandle != 0L) {
            nativeDetachSurface(nativeHandle)
        }
    }

    override fun onTouchEvent(event: MotionEvent): Boolean {
        scaleDetector.onTouchEvent(event)

        when (event.actionMasked) {
            MotionEvent.ACTION_DOWN -> {
                dragging = true
                lastTouchX = event.x
                lastTouchY = event.y
                parent?.requestDisallowInterceptTouchEvent(true)
                return true
            }
            MotionEvent.ACTION_POINTER_DOWN -> {
                dragging = false
                return true
            }
            MotionEvent.ACTION_MOVE -> {
                if (!scaleDetector.isInProgress && dragging) {
                    val dx = event.x - lastTouchX
                    val dy = event.y - lastTouchY
                    lastTouchX = event.x
                    lastTouchY = event.y
                    panBy(dx, dy)
                }
                return true
            }
            MotionEvent.ACTION_UP, MotionEvent.ACTION_CANCEL -> {
                dragging = false
                parent?.requestDisallowInterceptTouchEvent(false)
                return true
            }
        }

        return true
    }

    override fun onDetachedFromWindow() {
        destroyRenderer()
        super.onDetachedFromWindow()
    }

    private fun panBy(dx: Float, dy: Float) {
        val worldSize = TILE_SIZE_PX * 2.0.pow(zoom)
        val lonPerPixel = 360.0 / worldSize
        val latPerPixel = 360.0 / worldSize
        setCamera(
            centerLat = centerLat + dy * latPerPixel,
            centerLon = centerLon - dx * lonPerPixel,
            zoom = zoom,
            bearing = bearing,
            pitch = pitch,
        )
    }

    private fun ensureRenderer(): Long {
        if (nativeHandle == 0L) {
            nativeHandle = nativeCreateRenderer(tileUrlTemplate, cacheDir)
        }
        return nativeHandle
    }

    private fun recreateRenderer() {
        val wasSurfaceReady = surfaceReady
        destroyRenderer()
        if (wasSurfaceReady && holder.surface.isValid) {
            surfaceCreated(holder)
        }
    }

    private fun destroyRenderer() {
        if (nativeHandle == 0L) return
        if (surfaceReady) {
            nativeDetachSurface(nativeHandle)
        }
        nativeDestroyRenderer(nativeHandle)
        nativeHandle = 0L
    }

    private fun pushResizeIfReady() {
        if (!surfaceReady || nativeHandle == 0L || surfaceWidth <= 0 || surfaceHeight <= 0) return
        nativeResize(nativeHandle, surfaceWidth, surfaceHeight, resources.displayMetrics.density)
    }

    private fun pushCamera() {
        if (nativeHandle == 0L) return
        nativeSetCamera(nativeHandle, centerLat, centerLon, zoom, bearing, pitch)
    }

    private fun wrapLongitude(value: Double): Double {
        var lon = value
        while (lon < -180.0) lon += 360.0
        while (lon > 180.0) lon -= 360.0
        return lon
    }

    private companion object {
        private const val DEFAULT_TILE_URL_TEMPLATE = "http://10.0.2.2:8080/tile/{z}/{x}/{y}.png"
        private const val DEFAULT_CACHE_DIR = "tile-cache"
        private const val TILE_SIZE_PX = 256.0
        private const val MIN_LAT = -85.05112878
        private const val MAX_LAT = 85.05112878
        private const val MIN_ZOOM = 0.0
        private const val MAX_ZOOM = 30.0
        private const val MAX_PITCH = 85.0

        init {
            System.loadLibrary("osm_tile_engine")
        }

        @JvmStatic private external fun nativeCreateRenderer(tileUrlTemplate: String, cacheDir: String): Long
        @JvmStatic private external fun nativeAttachSurface(ptr: Long, surface: Surface)
        @JvmStatic private external fun nativeResize(ptr: Long, widthPx: Int, heightPx: Int, density: Float)
        @JvmStatic private external fun nativeSetCamera(
            ptr: Long,
            centerLat: Double,
            centerLon: Double,
            zoom: Double,
            bearing: Double,
            pitch: Double,
        )
        @JvmStatic private external fun nativeDetachSurface(ptr: Long)
        @JvmStatic private external fun nativeDestroyRenderer(ptr: Long)
        @JvmStatic private external fun nativeAddTileLayer(
            ptr: Long,
            layerId: String,
            urlTemplate: String,
            zIndex: Int,
            opacity: Float,
        )
        @JvmStatic private external fun nativeRemoveLayer(ptr: Long, layerId: String)
        @JvmStatic private external fun nativeSetLayerVisible(ptr: Long, layerId: String, visible: Boolean)
        @JvmStatic private external fun nativeSetLayerOpacity(ptr: Long, layerId: String, opacity: Float)
    }
}
