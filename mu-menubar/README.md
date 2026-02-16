# Mu Menu Bar

macOS menu bar app for Mu music/podcast player.

## Features

### Playback Controls
- **Play/Pause/Resume** - Toggle playback state
- **Next/Previous Track** - Navigate between tracks
- **Seek Forward/Backward** - Jump 15 seconds (buttons ready, backend pending)
- **Speed Control** - Adjust playback speed (0.5x - 3.0x)

### Display
- Current track title
- Track position (X of Y)
- Playback state indicator
- Progress bar (placeholder)

### Quick Actions
- Open Raycast extension
- Quit app

## Installation

### Build from source

```bash
cd mu-menubar
swift build -c release
```

### Run

```bash
.build/release/MuMenuBar
```

Or copy to Applications:

```bash
cp .build/release/MuMenuBar /Applications/
open /Applications/MuMenuBar
```

## Requirements

- macOS 13.0+
- `mu` CLI installed at `/opt/homebrew/bin/mu`
- Mu daemon running (auto-starts on play)

## Usage

1. Launch MuMenuBar
2. Click the music note icon in menu bar
3. Control playback directly from the popup

## Architecture

- **Polling:** Status updates every 2 seconds
- **Commands:** Executes `mu` CLI via Process
- **UI:** SwiftUI native interface
- **Lightweight:** Menu bar only, no dock icon

## TODO

- [ ] Implement seek forward/backward 15s in backend
- [ ] Show real progress bar with episode progress
- [ ] Display podcast artwork
- [ ] Keyboard shortcuts
- [ ] Media keys support
- [ ] Now Playing integration (macOS control center)

## Pairing with Raycast

This menu bar app focuses on **playback control** while the Raycast extension handles **content browsing and management**:

- **Menu Bar:** Quick controls, always visible
- **Raycast:** Browse library, manage subscriptions, view stats
