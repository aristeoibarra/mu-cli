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
    
    var body: some View {
        VStack(spacing: 12) {
            // Track info
            VStack(spacing: 4) {
                if let track = player.status?.track {
                    Text(track)
                        .font(.headline)
                        .lineLimit(2)
                        .multilineTextAlignment(.center)
                    
                    if let status = player.status {
                        Text("Track \(status.track_index + 1) of \(status.total_tracks)")
                            .font(.caption)
                            .foregroundColor(.secondary)
                    }
                } else {
                    Text("No track playing")
                        .font(.headline)
                        .foregroundColor(.secondary)
                }
            }
            .frame(maxWidth: .infinity)
            .padding(.horizontal)
            
            Divider()
            
            // Progress bar placeholder (will implement later with episode progress)
            if player.status?.track != nil {
                ProgressView(value: player.progress)
                    .progressViewStyle(.linear)
                    .padding(.horizontal)
            }
            
            // Playback controls
            HStack(spacing: 20) {
                // Previous (seek backward)
                Button(action: { player.seekBackward() }) {
                    Image(systemName: "gobackward.15")
                        .font(.system(size: 18))
                }
                .buttonStyle(.plain)
                .help("Seek backward 15s")
                
                // Previous track
                Button(action: { player.previous() }) {
                    Image(systemName: "backward.fill")
                        .font(.system(size: 16))
                }
                .buttonStyle(.plain)
                .help("Previous track")
                
                // Play/Pause
                Button(action: { player.togglePlayPause() }) {
                    Image(systemName: playPauseIcon)
                        .font(.system(size: 24))
                }
                .buttonStyle(.plain)
                .help(playPauseHelp)
                
                // Next track
                Button(action: { player.next() }) {
                    Image(systemName: "forward.fill")
                        .font(.system(size: 16))
                }
                .buttonStyle(.plain)
                .help("Next track")
                
                // Next (seek forward)
                Button(action: { player.seekForward() }) {
                    Image(systemName: "goforward.15")
                        .font(.system(size: 18))
                }
                .buttonStyle(.plain)
                .help("Seek forward 15s")
            }
            .padding(.vertical, 8)
            
            Divider()
            
            // Speed control
            HStack {
                Text("Speed:")
                    .font(.caption)
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
                .frame(width: 80)
            }
            .padding(.horizontal)
            
            Divider()
            
            // Quick actions
            VStack(spacing: 8) {
                Button("Open Raycast...") {
                    NSWorkspace.shared.open(URL(string: "raycast://extensions/aristeoibarra/mu/podcasts")!)
                }
                .buttonStyle(.plain)
                
                Button("Quit") {
                    NSApplication.shared.terminate(nil)
                }
                .buttonStyle(.plain)
            }
            .font(.caption)
            .padding(.bottom, 8)
        }
        .frame(width: 300)
        .padding()
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
