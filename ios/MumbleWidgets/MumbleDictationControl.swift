import AppIntents
import SwiftUI
import WidgetKit

@available(iOS 18.0, *)
struct MumbleDictationControl: ControlWidget {
    static let kind = "com.kitlangton.hex.mobile.control.dictate"

    var body: some ControlWidgetConfiguration {
        StaticControlConfiguration(kind: Self.kind) {
            ControlWidgetButton(action: ToggleMumbleControlIntent()) {
                Label("Dictate", systemImage: "mic.fill")
                Text("Mumble")
            }
        }
        .displayName("Dictate with Mumble")
        .description("Starts or finishes on-device dictation and copies the text.")
    }
}
