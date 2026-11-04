import AppIntents
import Foundation

/// Shared between MumbleMobile and MumbleWidgets so Control Center, Lock Screen, and Action Button
/// controls can start or finish dictation in Mumble.
@available(iOS 18.0, *)
nonisolated struct ToggleMumbleControlIntent: AppIntent {
    static let title: LocalizedStringResource = "Dictate with Mumble"
    static let description = IntentDescription("Starts or finishes on-device dictation and copies the transcript.")
    @available(iOS 26.0, *)
    static let supportedModes: IntentModes = .foreground
    static let isDiscoverable = false

    init() {}

    func perform() async throws -> some IntentResult {
        await MumbleSystemCommandBus.shared.dispatch(.toggleDictation)
        return .result()
    }
}
