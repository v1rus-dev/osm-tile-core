package yegor.cheprasov.osmtileengine

import android.content.Context
import android.graphics.Color
import android.graphics.drawable.GradientDrawable
import android.util.AttributeSet
import android.util.TypedValue
import android.view.GestureDetector
import android.view.Gravity
import android.view.MotionEvent
import android.view.Choreographer
import android.view.ScaleGestureDetector
import android.view.SurfaceHolder
import android.view.SurfaceView
import android.view.ViewConfiguration
import android.widget.FrameLayout
import android.widget.LinearLayout
import android.widget.TextView
import androidx.annotation.Keep
import kotlin.math.ln
import kotlin.math.exp
import java.util.Locale

@Keep
class OsmMapView @JvmOverloads constructor(
    context: Context,
    attrs: AttributeSet? = null,
    defStyleAttr: Int = 0,
) : FrameLayout(context, attrs, defStyleAttr), SurfaceHolder.Callback {
    enum class FlingMode {
        INERTIAL,
        STOP_ON_RELEASE,
    }

    data class FlingConfig(
        val enabled: Boolean = true,
        val mode: FlingMode = FlingMode.INERTIAL,
        val friction: Float = 4.2f,
        val velocityMultiplier: Float = 1f,
        val maxDurationMs: Long = 1400L,
        val minVelocityThreshold: Float = 45f,
    )

    data class DoubleTapZoomConfig(
        val enabled: Boolean = true,
        val zoomDelta: Double = 2.0,
        val durationMs: Long = 180L,
    )

    private var surfaceReady = false
    private var surfaceWidth = 0
    private var surfaceHeight = 0
    private var lastTouchX = 0f
    private var lastTouchY = 0f
    private var downTouchX = 0f
    private var downTouchY = 0f
    private var dragging = false
    private var trackingTap = false
    private var suppressGestureUntilUp = false
    private var lastTapTimeMs = 0L
    private var lastTapX = 0f
    private var lastTapY = 0f
    private var flingConfig = FlingConfig()
    private var flingVelocityX = 0.0
    private var flingVelocityY = 0.0
    private var flingStartNs = 0L
    private var flingLastFrameNs = 0L
    private var flingRunning = false
    private var doubleTapZoomConfig = DoubleTapZoomConfig()

    private var tileUrlTemplate: String = ""
    private var cacheDir: String = context.filesDir.resolve(DEFAULT_CACHE_DIR).absolutePath
    private var defaultCamera = OsmCamera()
    private var zoomOverlayEnabled = false
    private var centerCoordinatesOverlayEnabled = false
    private var externalEngine: OsmTileEngine? = null
    private var externalController: OsmMapController? = null
    private var externalControllerManaged = false
    private var internalController: OsmMapController? = null
    private var boundController: OsmMapController? = null

    private val mapSurfaceView = SurfaceView(context)
    private val overlayContainer = LinearLayout(context)
    private val zoomOverlayView = buildOverlayTextView()
    private val centerCoordinatesOverlayView = buildOverlayTextView()
    private val viewConfiguration = ViewConfiguration.get(context)
    private val touchSlopSquared = viewConfiguration.scaledTouchSlop
        .toFloat()
        .let { it * it }
    private val doubleTapSlopSquared = viewConfiguration.scaledDoubleTapSlop
        .toFloat()
        .let { it * it }

    private val scaleDetector = ScaleGestureDetector(
        context,
        object : ScaleGestureDetector.SimpleOnScaleGestureListener() {
            override fun onScale(detector: ScaleGestureDetector): Boolean {
                zoomBy(detector.scaleFactor.toDouble(), detector.focusX, detector.focusY)
                return true
            }
        },
    )
    private val gestureDetector = GestureDetector(
        context,
        object : GestureDetector.SimpleOnGestureListener() {
            override fun onDown(e: MotionEvent): Boolean {
                stopFling()
                return true
            }

            override fun onFling(
                e1: MotionEvent?,
                e2: MotionEvent,
                velocityX: Float,
                velocityY: Float,
            ): Boolean {
                if (!flingConfig.enabled || flingConfig.mode != FlingMode.INERTIAL) return false
                if (suppressGestureUntilUp || trackingTap) return false
                startFling(velocityX, velocityY)
                return true
            }
        },
    )
    private val flingFrameCallback = Choreographer.FrameCallback { frameTimeNs ->
        stepFling(frameTimeNs)
    }

    init {
        mapSurfaceView.layoutParams = LayoutParams(
            LayoutParams.MATCH_PARENT,
            LayoutParams.MATCH_PARENT,
        )
        mapSurfaceView.holder.addCallback(this)
        mapSurfaceView.isFocusable = true
        mapSurfaceView.isClickable = true
        mapSurfaceView.setOnTouchListener { _, event -> handleTouchEvent(event) }
        addView(mapSurfaceView)

        overlayContainer.orientation = LinearLayout.VERTICAL
        overlayContainer.layoutParams = LayoutParams(
            LayoutParams.WRAP_CONTENT,
            LayoutParams.WRAP_CONTENT,
            Gravity.TOP or Gravity.START,
        ).apply {
            val margin = dpToPx(12)
            setMargins(margin, margin, margin, margin)
        }
        overlayContainer.isClickable = false
        overlayContainer.isFocusable = false
        overlayContainer.addView(zoomOverlayView)
        overlayContainer.addView(centerCoordinatesOverlayView)
        addView(overlayContainer)

        isFocusable = true
        isClickable = true
        updateOverlayContent()
        updateOverlayVisibility()
    }

    fun setTileUrlTemplate(template: String) {
        if (tileUrlTemplate == template) return
        tileUrlTemplate = template
        internalController?.updateRendererConfig(tileUrlTemplate, cacheDir)
    }

    fun setCacheDir(path: String) {
        if (cacheDir == path) return
        cacheDir = path
        internalController?.updateRendererConfig(tileUrlTemplate, cacheDir)
    }

    fun setEngine(engine: OsmTileEngine?) {
        val controller = engine?.let { OsmMapController(it) }
        setExternalBinding(
            controller = controller,
            managed = controller != null,
            engine = engine,
        )
    }

    fun getEngine(): OsmTileEngine? = getController()?.getEngine()

    fun setController(controller: OsmMapController?) {
        setExternalBinding(
            controller = controller,
            managed = false,
            engine = null,
        )
    }

    private fun setExternalBinding(
        controller: OsmMapController?,
        managed: Boolean,
        engine: OsmTileEngine?,
    ) {
        if (
            externalController === controller &&
            externalControllerManaged == managed &&
            externalEngine === engine
        ) {
            return
        }

        val previousExternal = externalController
        val previousExternalManaged = externalControllerManaged
        externalController = controller
        externalControllerManaged = managed
        externalEngine = engine

        if (controller == null) {
            if (previousExternal != null) {
                if (boundController === previousExternal) {
                    unbindController(
                        previousExternal,
                        destroy = previousExternalManaged,
                    )
                } else {
                    previousExternal.removeInternalCameraObserver(this)
                    previousExternal.detachFromView(this)
                    if (previousExternalManaged) {
                        previousExternal.destroy()
                    }
                }
            }
            bindController(ensureInternalController())
            return
        }

        if (previousExternal != null &&
            previousExternal !== controller &&
            boundController !== previousExternal
        ) {
            previousExternal.removeInternalCameraObserver(this)
            previousExternal.detachFromView(this)
            if (previousExternalManaged) {
                previousExternal.destroy()
            }
        }

        boundController?.takeIf { it !== controller }?.let { existing ->
            val destroy = existing === internalController ||
                (existing === previousExternal && previousExternalManaged)
            unbindController(existing, destroy)
            if (destroy) {
                if (existing === internalController) {
                    internalController = null
                }
            }
        }

        internalController?.takeIf { it !== controller }?.let { existing ->
            if (existing !== boundController) {
                existing.destroy()
            }
            internalController = null
        }

        bindController(controller)
    }

    fun getController(): OsmMapController? = boundController ?: externalController ?: internalController

    fun getCamera(): OsmCamera = boundController?.getCamera() ?: defaultCamera

    fun setCamera(
        centerLat: Double,
        centerLon: Double,
        zoom: Double,
        bearing: Double = 0.0,
        pitch: Double = 0.0,
    ) {
        val camera = OsmCamera(centerLat, centerLon, zoom, bearing, pitch).normalized()
        defaultCamera = camera
        ensureActiveController().setCamera(camera, CameraChangeReason.PROGRAMMATIC)
    }

    fun moveCamera(
        centerLat: Double,
        centerLon: Double,
        zoom: Double,
        bearing: Double = 0.0,
        pitch: Double = 0.0,
    ) {
        val camera = OsmCamera(centerLat, centerLon, zoom, bearing, pitch).normalized()
        defaultCamera = camera
        ensureActiveController().moveCamera(camera)
    }

    fun moveCamera(camera: OsmCamera) {
        val normalized = camera.normalized()
        defaultCamera = normalized
        ensureActiveController().moveCamera(normalized)
    }

    fun animateCamera(
        centerLat: Double,
        centerLon: Double,
        zoom: Double,
        bearing: Double = 0.0,
        pitch: Double = 0.0,
        durationMs: Long = 300L,
    ) {
        val camera = OsmCamera(centerLat, centerLon, zoom, bearing, pitch).normalized()
        defaultCamera = camera
        ensureActiveController().animateCamera(camera, durationMs)
    }

    fun animateCamera(camera: OsmCamera, durationMs: Long = 300L) {
        val normalized = camera.normalized()
        defaultCamera = normalized
        ensureActiveController().animateCamera(normalized, durationMs)
    }

    fun fitBounds(bounds: OsmBounds, paddingDp: Float = 0f) {
        ensureActiveController().fitBounds(bounds, paddingDp)
    }

    fun fitBounds(bounds: OsmBounds, paddingDp: Float = 0f, durationMs: Long) {
        ensureActiveController().fitBounds(bounds, paddingDp, durationMs)
    }

    fun fitBounds(coordinates: Iterable<OsmCoordinate>, paddingDp: Float = 0f) {
        ensureActiveController().fitBounds(coordinates, paddingDp)
    }

    fun fitBounds(
        coordinates: Iterable<OsmCoordinate>,
        paddingDp: Float = 0f,
        durationMs: Long,
    ) {
        ensureActiveController().fitBounds(coordinates, paddingDp, durationMs)
    }

    fun fitBounds(markers: List<MobileMarker>, paddingDp: Float = 0f) {
        ensureActiveController().fitBounds(markers, paddingDp)
    }

    fun fitBounds(markers: List<MobileMarker>, paddingDp: Float = 0f, durationMs: Long) {
        ensureActiveController().fitBounds(markers, paddingDp, durationMs)
    }

    fun stopAnimation() {
        getController()?.stopAnimation()
    }

    fun addTileLayer(
        layerId: String,
        urlTemplate: String? = null,
        zIndex: Int = 0,
        opacity: Float = 1f,
    ) {
        ensureActiveController().addTileLayer(layerId, urlTemplate, zIndex, opacity)
    }

    fun removeLayer(layerId: String) {
        ensureActiveController().removeLayer(layerId)
    }

    fun setLayerVisible(layerId: String, visible: Boolean) {
        ensureActiveController().setLayerVisible(layerId, visible)
    }

    fun setLayerOpacity(layerId: String, opacity: Float) {
        ensureActiveController().setLayerOpacity(layerId, opacity)
    }

    fun setZoomOverlayEnabled(enabled: Boolean) {
        if (zoomOverlayEnabled == enabled) return
        zoomOverlayEnabled = enabled
        updateOverlayVisibility()
    }

    fun setCenterCoordinatesOverlayEnabled(enabled: Boolean) {
        if (centerCoordinatesOverlayEnabled == enabled) return
        centerCoordinatesOverlayEnabled = enabled
        updateOverlayVisibility()
    }

    override fun surfaceCreated(holder: SurfaceHolder) {
        surfaceReady = true
        ensureActiveController().onSurfaceCreated(holder.surface)
    }

    override fun surfaceChanged(holder: SurfaceHolder, format: Int, width: Int, height: Int) {
        surfaceWidth = width
        surfaceHeight = height
        ensureActiveController().onSurfaceChanged(
            width,
            height,
            resources.displayMetrics.density,
        )
    }

    override fun surfaceDestroyed(holder: SurfaceHolder) {
        surfaceReady = false
        boundController?.onSurfaceDestroyed()
    }

    override fun onTouchEvent(event: MotionEvent): Boolean {
        return handleTouchEvent(event)
    }

    override fun onDetachedFromWindow() {
        boundController?.let { controller ->
            controller.removeInternalCameraObserver(this)
            if (surfaceReady) {
                controller.onSurfaceDestroyed()
            }
            if (controller === internalController || (controller === externalController && externalControllerManaged)) {
                controller.detachFromView(this)
                controller.destroy()
                boundController = null
                if (controller === internalController) {
                    internalController = null
                }
            } else {
                boundController = null
            }
        }
        super.onDetachedFromWindow()
    }

    private fun handleTouchEvent(event: MotionEvent): Boolean {
        if (event.actionMasked == MotionEvent.ACTION_DOWN) {
            return handleActionDown(event)
        }

        if (!suppressGestureUntilUp) {
            gestureDetector.onTouchEvent(event)
            scaleDetector.onTouchEvent(event)
        }

        when (event.actionMasked) {
            MotionEvent.ACTION_POINTER_DOWN -> {
                trackingTap = false
                dragging = false
                return true
            }
            MotionEvent.ACTION_MOVE -> {
                handleActionMove(event)
                return true
            }
            MotionEvent.ACTION_UP -> {
                handleActionUp(event)
                if (flingConfig.mode == FlingMode.STOP_ON_RELEASE) {
                    stopFling()
                }
                this.parent?.requestDisallowInterceptTouchEvent(false)
                return true
            }
            MotionEvent.ACTION_CANCEL -> {
                resetActiveTouch()
                clearLastTap()
                this.parent?.requestDisallowInterceptTouchEvent(false)
                return true
            }
        }

        return true
    }

    private fun handleActionDown(event: MotionEvent): Boolean {
        stopFling()
        downTouchX = event.x
        downTouchY = event.y
        lastTouchX = event.x
        lastTouchY = event.y
        dragging = false
        trackingTap = true
        suppressGestureUntilUp = false
        this.parent?.requestDisallowInterceptTouchEvent(true)

        if (isDoubleTap(event)) {
            resetActiveTouch()
            clearLastTap()
            suppressGestureUntilUp = true
            zoomInOnDoubleTap(event.x, event.y)
            return true
        }

        gestureDetector.onTouchEvent(event)
        scaleDetector.onTouchEvent(event)
        return true
    }

    private fun handleActionMove(event: MotionEvent) {
        if (suppressGestureUntilUp || scaleDetector.isInProgress || event.pointerCount > 1) {
            trackingTap = false
            dragging = false
            return
        }

        if (!dragging) {
            if (!trackingTap) {
                return
            }
            val dxFromDown = event.x - downTouchX
            val dyFromDown = event.y - downTouchY
            if (distanceSquared(dxFromDown, dyFromDown) <= touchSlopSquared) {
                return
            }
            dragging = true
            trackingTap = false
            lastTouchX = event.x
            lastTouchY = event.y
            return
        }

        val dx = event.x - lastTouchX
        val dy = event.y - lastTouchY
        lastTouchX = event.x
        lastTouchY = event.y
        panBy(dx, dy)
    }

    private fun handleActionUp(event: MotionEvent) {
        if (suppressGestureUntilUp) {
            resetActiveTouch()
            return
        }

        if (trackingTap && !dragging && isWithinTouchSlop(event.x, event.y)) {
            lastTapTimeMs = event.eventTime
            lastTapX = downTouchX
            lastTapY = downTouchY
        }
        resetActiveTouch()
    }

    private fun resetActiveTouch() {
        dragging = false
        trackingTap = false
        suppressGestureUntilUp = false
    }

    private fun isDoubleTap(event: MotionEvent): Boolean {
        if (!doubleTapZoomConfig.enabled || lastTapTimeMs == 0L) return false
        val elapsedMs = event.eventTime - lastTapTimeMs
        if (elapsedMs < 0L || elapsedMs > ViewConfiguration.getDoubleTapTimeout().toLong()) return false

        return distanceSquared(event.x - lastTapX, event.y - lastTapY) <= doubleTapSlopSquared
    }

    private fun isWithinTouchSlop(x: Float, y: Float): Boolean =
        distanceSquared(x - downTouchX, y - downTouchY) <= touchSlopSquared

    private fun distanceSquared(dx: Float, dy: Float): Float = dx * dx + dy * dy

    private fun clearLastTap() {
        lastTapTimeMs = 0L
    }

    private fun panBy(dx: Float, dy: Float) {
        if (surfaceWidth <= 0 || surfaceHeight <= 0) return

        val controller = ensureActiveController()
        val camera = controller.getCamera()
        val worldSize = worldSizePx(camera.zoom)
        val centerWorldX = longitudeToNormalizedX(camera.centerLon) * worldSize
        val centerWorldY = latitudeToNormalizedY(camera.centerLat) * worldSize
        val nextWorldX = wrapWorldCoordinate(centerWorldX - dx, worldSize)
        val nextWorldY = (centerWorldY - dy).coerceIn(0.0, worldSize)
        controller.setCamera(
            camera.copy(
                centerLat = normalizedYToLatitude(nextWorldY / worldSize),
                centerLon = normalizedXToLongitude(nextWorldX / worldSize),
            ),
            CameraChangeReason.GESTURE,
        )
    }

    fun setFlingConfig(config: FlingConfig) {
        flingConfig = config
        if (!config.enabled) {
            stopFling()
        }
    }

    fun getFlingConfig(): FlingConfig = flingConfig

    fun setDoubleTapZoomConfig(config: DoubleTapZoomConfig) {
        require(config.zoomDelta.isFinite()) { "Double-tap zoom delta must be finite" }
        require(config.zoomDelta >= 0.0) { "Double-tap zoom delta must be non-negative" }
        require(config.durationMs >= 0L) { "Double-tap zoom duration must be non-negative" }
        doubleTapZoomConfig = config
    }

    fun getDoubleTapZoomConfig(): DoubleTapZoomConfig = doubleTapZoomConfig

    fun setDoubleTapZoomDelta(zoomDelta: Double) {
        setDoubleTapZoomConfig(doubleTapZoomConfig.copy(zoomDelta = zoomDelta))
    }

    fun getDoubleTapZoomDelta(): Double = doubleTapZoomConfig.zoomDelta

    private fun startFling(velocityX: Float, velocityY: Float) {
        val speedX = velocityX * flingConfig.velocityMultiplier
        val speedY = velocityY * flingConfig.velocityMultiplier
        val speed = kotlin.math.hypot(speedX.toDouble(), speedY.toDouble()).toFloat()
        if (speed < flingConfig.minVelocityThreshold) return
        stopFling()
        flingVelocityX = speedX.toDouble()
        flingVelocityY = speedY.toDouble()
        flingStartNs = 0L
        flingLastFrameNs = 0L
        flingRunning = true
        Choreographer.getInstance().postFrameCallback(flingFrameCallback)
    }

    private fun stopFling() {
        if (!flingRunning) return
        flingRunning = false
        Choreographer.getInstance().removeFrameCallback(flingFrameCallback)
    }

    private fun stepFling(frameTimeNs: Long) {
        if (!flingRunning || surfaceWidth <= 0 || surfaceHeight <= 0) {
            stopFling()
            return
        }
        if (flingStartNs == 0L) {
            flingStartNs = frameTimeNs
            flingLastFrameNs = frameTimeNs
            Choreographer.getInstance().postFrameCallback(flingFrameCallback)
            return
        }
        val dtSec = ((frameTimeNs - flingLastFrameNs).coerceAtLeast(0L)).toDouble() / 1_000_000_000.0
        flingLastFrameNs = frameTimeNs
        val elapsedMs = (frameTimeNs - flingStartNs) / 1_000_000L

        val decay = exp((-flingConfig.friction * dtSec).toDouble())
        flingVelocityX *= decay
        flingVelocityY *= decay

        val dx = (flingVelocityX * dtSec).toFloat()
        val dy = (flingVelocityY * dtSec).toFloat()
        panBy(dx, dy)

        val remainingSpeed = kotlin.math.hypot(flingVelocityX, flingVelocityY).toFloat()
        if (remainingSpeed < flingConfig.minVelocityThreshold || elapsedMs >= flingConfig.maxDurationMs) {
            stopFling()
            return
        }
        Choreographer.getInstance().postFrameCallback(flingFrameCallback)
    }

    private fun zoomBy(scaleFactor: Double, focusX: Float, focusY: Float) {
        if (surfaceWidth <= 0 || surfaceHeight <= 0) return

        val zoomDelta = ln(scaleFactor) / ln(2.0)
        if (!zoomDelta.isFinite() || zoomDelta == 0.0) return

        val controller = ensureActiveController()
        val camera = controller.getCamera()
        val newZoom = (camera.zoom + zoomDelta).coerceIn(MIN_ZOOM, MAX_ZOOM)
        if (newZoom == camera.zoom) return

        val oldWorldSize = worldSizePx(camera.zoom)
        val newWorldSize = worldSizePx(newZoom)
        val halfWidth = surfaceWidth / 2.0
        val halfHeight = surfaceHeight / 2.0
        val centerNormX = longitudeToNormalizedX(camera.centerLon)
        val centerNormY = latitudeToNormalizedY(camera.centerLat)
        val focusNormX = (centerNormX * oldWorldSize + (focusX - halfWidth)) / oldWorldSize
        val focusNormY = (centerNormY * oldWorldSize + (focusY - halfHeight)) / oldWorldSize
        val nextCenterNormX =
            (focusNormX * newWorldSize - (focusX - halfWidth)) / newWorldSize
        val nextCenterNormY =
            ((focusNormY * newWorldSize - (focusY - halfHeight)) / newWorldSize).coerceIn(0.0, 1.0)

        controller.setCamera(
            camera.copy(
                centerLat = normalizedYToLatitude(nextCenterNormY),
                centerLon = normalizedXToLongitude(nextCenterNormX),
                zoom = newZoom,
            ),
            CameraChangeReason.GESTURE,
        )
    }

    private fun zoomInOnDoubleTap(tapX: Float, tapY: Float) {
        if (surfaceWidth <= 0 || surfaceHeight <= 0) return

        val controller = ensureActiveController()
        val camera = controller.getCamera()
        val oldWorldSize = worldSizePx(camera.zoom)
        val halfWidth = surfaceWidth / 2.0
        val halfHeight = surfaceHeight / 2.0
        val tapNormX =
            (longitudeToNormalizedX(camera.centerLon) * oldWorldSize + (tapX - halfWidth)) /
                oldWorldSize
        val tapNormY =
            ((latitudeToNormalizedY(camera.centerLat) * oldWorldSize + (tapY - halfHeight)) /
                oldWorldSize).coerceIn(0.0, 1.0)
        val targetZoom = (camera.zoom + doubleTapZoomConfig.zoomDelta).coerceIn(MIN_ZOOM, MAX_ZOOM)

        controller.animateCamera(
            camera.copy(
                centerLat = normalizedYToLatitude(tapNormY),
                centerLon = normalizedXToLongitude(tapNormX),
                zoom = targetZoom,
            ),
            doubleTapZoomConfig.durationMs,
            CameraChangeReason.GESTURE,
        )
    }

    private fun ensureActiveController(): OsmMapController {
        val controller = externalController ?: ensureInternalController()
        bindController(controller)
        return controller
    }

    private fun ensureInternalController(): OsmMapController {
        val existing = internalController
        if (existing != null) {
            return existing
        }

        return OsmMapController(tileUrlTemplate, cacheDir).also { controller ->
            internalController = controller
            controller.setCamera(defaultCamera, CameraChangeReason.RESTORE)
        }
    }

    private fun bindController(controller: OsmMapController) {
        if (boundController === controller) {
            updateOverlayContent(controller.getCamera())
            return
        }

        controller.attachToView(this)
        controller.addInternalCameraObserver(this) { camera, _ ->
            defaultCamera = camera
            updateOverlayContent(camera)
        }
        boundController = controller
        updateOverlayContent(controller.getCamera())

        if (surfaceReady && mapSurfaceView.holder.surface.isValid) {
            controller.onSurfaceCreated(mapSurfaceView.holder.surface)
        }
        if (surfaceWidth > 0 && surfaceHeight > 0) {
            controller.onSurfaceChanged(
                surfaceWidth,
                surfaceHeight,
                resources.displayMetrics.density,
            )
        }
    }

    private fun unbindController(controller: OsmMapController, destroy: Boolean) {
        controller.removeInternalCameraObserver(this)
        if (surfaceReady) {
            controller.onSurfaceDestroyed()
        }
        controller.detachFromView(this)
        if (boundController === controller) {
            boundController = null
        }
        if (destroy) {
            controller.destroy()
        }
    }

    internal fun onControllerDestroyed(controller: OsmMapController) {
        if (boundController === controller) {
            boundController = null
        }
        if (externalController === controller) {
            externalController = null
            externalControllerManaged = false
            externalEngine = null
        }
        if (internalController === controller) {
            internalController = null
        }
        updateOverlayContent(defaultCamera)
    }

    private fun buildOverlayTextView(): TextView =
        TextView(context).apply {
            setTextColor(Color.WHITE)
            setTextSize(TypedValue.COMPLEX_UNIT_SP, 13f)
            setPadding(dpToPx(10), dpToPx(6), dpToPx(10), dpToPx(6))
            layoutParams = LinearLayout.LayoutParams(
                LayoutParams.WRAP_CONTENT,
                LayoutParams.WRAP_CONTENT,
            ).apply {
                bottomMargin = dpToPx(6)
            }
            background = GradientDrawable().apply {
                shape = GradientDrawable.RECTANGLE
                cornerRadius = dpToPx(8).toFloat()
                setColor(Color.argb(170, 22, 26, 33))
            }
        }

    private fun updateOverlayVisibility() {
        zoomOverlayView.visibility = if (zoomOverlayEnabled) VISIBLE else GONE
        centerCoordinatesOverlayView.visibility = if (centerCoordinatesOverlayEnabled) VISIBLE else GONE
        overlayContainer.visibility =
            if (zoomOverlayEnabled || centerCoordinatesOverlayEnabled) VISIBLE else GONE
    }

    private fun updateOverlayContent(camera: OsmCamera = getCamera()) {
        zoomOverlayView.text = "Zoom ${formatZoom(camera.zoom)}"
        centerCoordinatesOverlayView.text =
            "${formatCoordinate(camera.centerLat)}, ${formatCoordinate(camera.centerLon)}"
    }

    private fun formatZoom(value: Double): String = String.format(Locale.US, "%.2f", value)

    private fun formatCoordinate(value: Double): String = String.format(Locale.US, "%.5f", value)

    private fun dpToPx(value: Int): Int =
        TypedValue.applyDimension(
            TypedValue.COMPLEX_UNIT_DIP,
            value.toFloat(),
            resources.displayMetrics,
        ).toInt()

}
