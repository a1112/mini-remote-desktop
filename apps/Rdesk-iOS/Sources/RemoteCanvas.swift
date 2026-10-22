import SwiftUI
import UIKit

struct RemoteCanvas: UIViewRepresentable {
    let image: UIImage?
    let enabled: Bool
    let pointer: (CGPoint, String) -> Void

    func makeUIView(context: Context) -> RemoteImageView { RemoteImageView() }

    func updateUIView(_ view: RemoteImageView, context: Context) {
        if !enabled { view.releasePointer() }
        view.pointer = pointer
        view.image = image
        view.isUserInteractionEnabled = enabled
    }

    static func dismantleUIView(_ view: RemoteImageView, coordinator: ()) {
        view.releasePointer()
    }
}

final class RemoteImageView: UIImageView {
    var pointer: ((CGPoint, String) -> Void)?
    private var heldTouch: UITouch?
    private var lastPoint: CGPoint?
    private var previousSize = CGSize.zero

    // A decoded desktop's pixel size must never enlarge the phone's layout.
    override var intrinsicContentSize: CGSize {
        CGSize(width: UIView.noIntrinsicMetric, height: UIView.noIntrinsicMetric)
    }

    init() {
        super.init(frame: .zero)
        contentMode = .scaleAspectFit
        backgroundColor = .black
        isMultipleTouchEnabled = false
        accessibilityIdentifier = "remoteCanvas"
        isAccessibilityElement = true
        accessibilityLabel = "远程电脑画面，轻触点击，按住拖动"
        NotificationCenter.default.addObserver(self, selector: #selector(releasePointer),
            name: UIApplication.willResignActiveNotification, object: nil)
    }

    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    deinit { NotificationCenter.default.removeObserver(self) }

    private func normalized(_ touch: UITouch, clamp: Bool) -> CGPoint? {
        guard let image else { return nil }
        return RemoteGeometry.normalized(touch.location(in: self),
            in: RemoteGeometry.imageRect(image: image.size, bounds: bounds), clamp: clamp)
    }

    override func touchesBegan(_ touches: Set<UITouch>, with event: UIEvent?) {
        guard heldTouch == nil, let touch = touches.first, let point = normalized(touch, clamp: false) else { return }
        heldTouch = touch
        lastPoint = point
        pointer?(point, "down")
    }

    override func touchesMoved(_ touches: Set<UITouch>, with event: UIEvent?) {
        guard let touch = heldTouch, touches.contains(touch), let point = normalized(touch, clamp: true) else { return }
        lastPoint = point
        pointer?(point, "move")
    }

    override func touchesEnded(_ touches: Set<UITouch>, with event: UIEvent?) {
        guard let touch = heldTouch, touches.contains(touch) else { return }
        if let point = normalized(touch, clamp: true) { lastPoint = point }
        releasePointer()
    }

    override func touchesCancelled(_ touches: Set<UITouch>, with event: UIEvent?) { releasePointer() }
    override func layoutSubviews() {
        // Rotation or resizing must not leave the remote mouse held down.
        if bounds.size != previousSize {
            releasePointer()
            previousSize = bounds.size
        }
        super.layoutSubviews()
    }

    @objc func releasePointer() {
        if heldTouch != nil, let lastPoint { pointer?(lastPoint, "up") }
        heldTouch = nil
        lastPoint = nil
    }
}
