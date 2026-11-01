# What's Mojito
Mojito's a unified, opinianated shortcut app that revolve around turning caps lock key into the core part of controlling the machine. The goal of it is that user could archive their goal with less finger movement.

# Core feature
Mojito turn tapping caps lock into esc, but with hold press, it turn into meta key that could combine with other key to do specific things.

1. meta + hjkl = arrow key
2. meta + i = backspace
3. meta + enter = cycle input languages (caps lock in MacOS)
4. meta + m = app switch overlay (details below)

the key listed here is the absolute position in the keyboard, meaning that even if user change their keyboard layout or language, the shortcut must be the same.

# App Switching

When user pressing meta+m, the overlay will pop up to let user choose which app they want to switch to, this also mean that the window must be changed too if the app is on different window.

1. User list the apps they want to show in overlay in the Mojito setting app.
2. User could choose the app by using pre-defined key for that app.
3. User could press m again after meta + m to change the mode from pre-defined app to currently open apps with assigned label keys (not from the user defined key, but consistent and use the key that's easier to reach first using colemak-dh idea.

# Tech Stack

1. The app is written in Rust. Using https://github.com/anomalyco/rune as a ui framework for the app and overlay. Could use https://github.com/anomalyco/hex as an example for the framework.
2. Design it to feel like a native app for MacOS with Apple's current design system (liquid glass) with minimalistic approach of Hex, but prioritise the performance and resource usage.
3. The app will be use mostly on MacOS, so there's a technical challenge on cross platform compatibility, prefer one that work on MacOS first, but if possible, keep things universal so we could expand to other OS.
