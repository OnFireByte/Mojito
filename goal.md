# What's Mojito?

Mojito is a unified, opinionated shortcut app that revolves around turning the Caps Lock key into the core of machine control. Its goal is to help users accomplish tasks with less finger movement.

# Core Features

Mojito turns a tap of the Caps Lock key into Escape. When held down, however, it acts as a Meta key that can be combined with other keys to perform specific actions.

1. **Meta + H/J/K/L** = Arrow keys
2. **Meta + I** = Backspace
3. **Meta + Enter** = Cycle input languages (similar to Caps Lock on macOS)
4. **Meta + M** = Open the app-switching overlay (details below)

The keys listed above refer to their physical positions on the keyboard. This means that shortcuts must remain consistent regardless of the user's keyboard layout or input language.

# App Switching

When the user presses Meta + M, an overlay appears, allowing them to choose which app to switch to. Switching apps must also bring the correct window into focus, even if the app is running in a different window or workspace.

1. Users can configure which apps appear in the overlay through Mojito's settings app.
2. Users can select an app using a predefined key assigned to that app.
3. Pressing M again while holding Meta + M switches the overlay from the predefined app list to a list of currently open apps. Each app is assigned a label key automatically. These keys are not based on user-defined assignments; instead, they follow a consistent layout that prioritizes the easiest-to-reach keys, taking inspiration from the Colemak-DH layout.

# Tech Stack

1. The app is written in Rust, using [Rune](https://github.com/anomalyco/rune) as the UI framework for both the main application and the overlay. [Hex](https://github.com/anomalyco/hex) can serve as a reference implementation for the framework.
2. The design should feel like a native macOS app, following Apple's current design language (Liquid Glass) while maintaining the minimalistic aesthetic of Hex. However, performance and resource efficiency should take priority over visual effects.
3. The app will primarily target macOS, which introduces some challenges for cross-platform compatibility. Prioritize macOS support first, but keep the architecture as platform-agnostic as reasonably possible to allow for potential expansion to other operating systems.