import SwiftUI

struct PlaybackStatus: Codable {
    let playing: Bool
    let paused: Bool
    let track: String?
    let track_index: Int
    let total_tracks: Int
}

struct MenuBarView: View {
    @StateObject var player = PlayerController()
    @State private var isDragging = false
    @State private var dragProgress: Double = 0.0
    
    var body: some View {
        VStack(spacing: 16) {
            // Track info
            VStack(spacing: 4) {
                if let track = player.status?.track {
                    Text(track)
                        .font(.system(size: 13, weight: .semibold))
                        .lineLimit(2)
                        .multilineTextAlignment(.center)
                        .fixedSize(horizontal: false, vertical: true)
                    
                    if let status = player.status {
                        Text("Track \(status.track_index + 1) of \(status.total_tracks)")
                            .font(.system(size: 11))
                            .foregroundColor(.secondary)
                    }
                } else {
                    Text("Not playing")
                        .font(.system(size: 13))
                        .foregroundColor(.secondary)
                }
            }
            .frame(maxWidth: .infinity)
            .padding(.horizontal, 20)
            .padding(.top, 8)
            
            // Progress bar with time labels (Spotify style)
            if player.status?.track != nil {
                VStack(spacing: 4) {
                    // Draggable progress bar
                    GeometryReader { geometry in
                        ZStack(alignment: .leading) {
                            // Background track
                            Rectangle()
                                .fill(Color.gray.opacity(0.3))
                                .frame(height: 4)
                                .cornerRadius(2)
                            
                            // Progress track
                            Rectangle()
                                .fill(Color.accentColor)
                                .frame(width: geometry.size.width * (isDragging ? dragProgress : player.progress), height: 4)
                                .cornerRadius(2)
                        }
                        .frame(height: 4)
                        .gesture(
                            DragGesture(minimumDistance: 0)
                                .onChanged { value in
                                    isDragging = true
                                    dragProgress = min(max(0, value.location.x / geometry.size.width), 1.0)
                                }
                                .onEnded { value in
                                    let finalProgress = min(max(0, value.location.x / geometry.size.width), 1.0)
                                    player.seek(to: finalProgress)
                                    isDragging = false
                                }
                        )
                        .onHover { hovering in
                            if hovering {
                                NSCursor.pointingHand.push()
                            } else {
                                NSCursor.pop()
                            }
                        }
                    }
                    .frame(height: 4)
                    .padding(.horizontal, 20)
                    
                    // Time labels
                    HStack {
                        Text(formatTime(player.currentTime))
                            .font(.system(size: 10, design: .monospaced))
                            .foregroundColor(.secondary)
                        
                        Spacer()
                        
                        Text(formatTime(player.duration))
                            .font(.system(size: 10, design: .monospaced))
                            .foregroundColor(.secondary)
                    }
                    .padding(.horizontal, 20)
                }
            }
            
            // Playback controls
            HStack(spacing: 24) {
                // Seek backward
                Button(action: { player.seekBackward() }) {
                    Image(systemName: "gobackward.15")
                        .font(.system(size: 20))
                        .foregroundColor(.primary)
                }
                .buttonStyle(.plain)
                .help("Seek backward 15s")
                
                // Previous track
                Button(action: { player.previous() }) {
                    Image(systemName: "backward.fill")
                        .font(.system(size: 18))
                        .foregroundColor(.primary)
                }
                .buttonStyle(.plain)
                .help("Previous track")
                
                // Play/Pause (larger, centered)
                Button(action: { player.togglePlayPause() }) {
                    ZStack {
                        Circle()
                            .fill(Color.accentColor)
                            .frame(width: 44, height: 44)
                        
                        Image(systemName: playPauseIcon)
                            .font(.system(size: 18))
                            .foregroundColor(.white)
                    }
                }
                .buttonStyle(.plain)
                .help(playPauseHelp)
                
                // Next track
                Button(action: { player.next() }) {
                    Image(systemName: "forward.fill")
                        .font(.system(size: 18))
                        .foregroundColor(.primary)
                }
                .buttonStyle(.plain)
                .help("Next track")
                
                // Seek forward
                Button(action: { player.seekForward() }) {
                    Image(systemName: "goforward.15")
                        .font(.system(size: 20))
                        .foregroundColor(.primary)
                }
                .buttonStyle(.plain)
                .help("Seek forward 15s")
            }
            .padding(.vertical, 12)
            
            // Speed control (compact)
            HStack(spacing: 8) {
                Image(systemName: "gauge.medium")
                    .font(.system(size: 12))
                    .foregroundColor(.secondary)
                
                Picker("", selection: $player.speed) {
                    Text("0.5x").tag(0.5)
                    Text("0.75x").tag(0.75)
                    Text("1.0x").tag(1.0)
                    Text("1.25x").tag(1.25)
                    Text("1.5x").tag(1.5)
                    Text("1.75x").tag(1.75)
                    Text("2.0x").tag(2.0)
                    Text("2.5x").tag(2.5)
                    Text("3.0x").tag(3.0)
                }
                .pickerStyle(.menu)
                .frame(width: 75)
            }
            .padding(.horizontal, 20)
            .padding(.bottom, 12)
        }
        .frame(width: 320)
        .background(Color(NSColor.windowBackgroundColor))
    }
    
    private func formatTime(_ seconds: Double) -> String {
        let mins = Int(seconds) / 60
        let secs = Int(seconds) % 60
        return String(format: "%d:%02d", mins, secs)
    }
    
    private var playPauseIcon: String {
        guard let status = player.status else { return "play.fill" }
        if status.playing { return "pause.fill" }
        if status.paused { return "play.fill" }
        return "play.fill"
    }
    
    private var playPauseHelp: String {
        guard let status = player.status else { return "Play" }
        if status.playing { return "Pause" }
        if status.paused { return "Resume" }
        return "Play"
    }
}

class PlayerController: ObservableObject {
    @Published var status: PlaybackStatus?
    @Published var speed: Double = 1.0 {
        didSet {
            setSpeed(speed)
        }
    }
    @Published var progress: Double = 0.0
    @Published var currentTime: Double = 0.0
    @Published var duration: Double = 100.0
    
    private var timer: Timer?
    private var wasPlaying = false
    weak var appDelegate: AppDelegate?
    
    init() {
        startPolling()
    }
    
    deinit {
        timer?.invalidate()
    }
    
    private func startPolling() {
        // Poll status every 2 seconds
        timer = Timer.scheduledTimer(withTimeInterval: 2.0, repeats: true) { [weak self] _ in
            self?.updateStatus()
        }
        timer?.fire() // Run immediately
    }
    
    private func updateStatus() {
        let newStatus: PlaybackStatus? = runCommand("status")
        
        // Auto-show popover when playback starts
        if let newStatus = newStatus, newStatus.playing && !wasPlaying {
            DispatchQueue.main.async { [weak self] in
                self?.appDelegate?.showPopover()
            }
        }
        
        // Update progress (for now, simulate based on track index)
        if let newStatus = newStatus, newStatus.total_tracks > 0 {
            let trackProgress = Double(newStatus.track_index) / Double(newStatus.total_tracks)
            progress = trackProgress
            
            // Simulate current time and duration (will be real from backend later)
            currentTime = trackProgress * duration
        }
        
        wasPlaying = newStatus?.playing ?? false
        status = newStatus
    }
    
    func togglePlayPause() {
        guard let status = status else { return }
        
        if status.playing {
            _ = runCommand("pause") as PlaybackStatus?
        } else if status.paused {
            _ = runCommand("resume") as PlaybackStatus?
        }
        
        // Update immediately
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.3) {
            self.updateStatus()
        }
    }
    
    func next() {
        _ = runCommand("next") as PlaybackStatus?
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.3) {
            self.updateStatus()
        }
    }
    
    func previous() {
        _ = runCommand("previous") as PlaybackStatus?
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.3) {
            self.updateStatus()
        }
    }
    
    func seekForward() {
        let task = Process()
        task.launchPath = "/opt/homebrew/bin/mu"
        task.arguments = ["seek", "15"]
        
        do {
            try task.run()
        } catch {
            print("Failed to seek forward: \(error)")
        }
    }
    
    func seekBackward() {
        let task = Process()
        task.launchPath = "/opt/homebrew/bin/mu"
        task.arguments = ["seek", "-15"]
        
        do {
            try task.run()
        } catch {
            print("Failed to seek backward: \(error)")
        }
    }
    
    func seek(to progress: Double) {
        // Calculate seconds based on progress and duration
        let seconds = Int(progress * duration)
        // TODO: Implement absolute seek in backend
        print("Seek to \(seconds)s (progress: \(progress))")
    }
    
    private func setSpeed(_ speed: Double) {
        let task = Process()
        task.launchPath = "/opt/homebrew/bin/mu"
        task.arguments = ["speed", "\(speed)"]
        
        do {
            try task.run()
        } catch {
            print("Failed to set speed: \(error)")
        }
    }
    
    private func runCommand<T: Codable>(_ command: String) -> T? {
        let task = Process()
        let pipe = Pipe()
        
        task.standardOutput = pipe
        task.standardError = pipe
        task.launchPath = "/opt/homebrew/bin/mu"
        task.arguments = [command]
        
        do {
            try task.run()
            task.waitUntilExit()
            
            let data = pipe.fileHandleForReading.readDataToEndOfFile()
            let decoder = JSONDecoder()
            return try? decoder.decode(T.self, from: data)
        } catch {
            return nil
        }
    }
}
