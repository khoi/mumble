import SwiftUI
import WidgetKit

@main
struct MumbleWidgetsBundle: WidgetBundle {
    var body: some Widget {
        MumbleDictationLiveActivity()
        if #available(iOS 18.0, *) {
            MumbleDictationControl()
        }
    }
}
