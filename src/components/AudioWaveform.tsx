import React, { useEffect, useRef, useState, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";

interface AudioWaveformProps {
  filePath: string;
  width?: number;
  height?: number;
}

/** Parse a standard WAV file from a Uint8Array. Returns PCM samples normalized to [-1, 1]. */
function parseWav(bytes: Uint8Array): Float32Array | null {
  const view = new DataView(bytes.buffer);
  // Minimum WAV header is 44 bytes; check RIFF and WAVE markers
  if (bytes.length < 44) return null;
  const riff = String.fromCharCode(...bytes.slice(0, 4));
  const wave = String.fromCharCode(...bytes.slice(8, 12));
  if (riff !== "RIFF" || wave !== "WAVE") return null;

  const numChannels = view.getUint16(22, true);
  const bitsPerSample = view.getUint16(34, true);
  const dataOffset = 44; // standard PCM WAV

  if (bitsPerSample !== 16 && bitsPerSample !== 8 && bitsPerSample !== 24) return null;

  const dataSize = bytes.length - dataOffset;
  const bytesPerSample = bitsPerSample / 8;
  const numSamples = Math.floor(dataSize / (bytesPerSample * numChannels));

  const pcm = new Float32Array(numSamples);
  for (let i = 0; i < numSamples; i++) {
    let sample = 0;
    const offset = dataOffset + i * bytesPerSample * numChannels;
    if (bitsPerSample === 16) {
      sample = view.getInt16(offset, true) / 32768;
    } else if (bitsPerSample === 8) {
      sample = (view.getUint8(offset) - 128) / 128;
    } else if (bitsPerSample === 24) {
      const lo = view.getUint8(offset);
      const mid = view.getUint8(offset + 1);
      const hi = view.getInt8(offset + 2);
      sample = ((hi << 16) | (mid << 8) | lo) / 8388608;
    }
    pcm[i] = sample;
  }
  return pcm;
}

/** Build a static placeholder sine-wave waveform. */
function buildPlaceholderWaveform(numBuckets: number): number[] {
  const result: number[] = [];
  for (let i = 0; i < numBuckets; i++) {
    result.push(
      0.4 * Math.abs(Math.sin((i / numBuckets) * Math.PI * 12)) +
        0.15 * Math.abs(Math.sin((i / numBuckets) * Math.PI * 31))
    );
  }
  return result;
}

/** Downsample PCM to bucket RMS values for display. */
function buildWaveformBuckets(pcm: Float32Array, numBuckets: number): number[] {
  const buckets: number[] = [];
  const bucketSize = Math.max(1, Math.floor(pcm.length / numBuckets));
  for (let b = 0; b < numBuckets; b++) {
    let sum = 0;
    const start = b * bucketSize;
    const end = Math.min(start + bucketSize, pcm.length);
    for (let i = start; i < end; i++) {
      sum += pcm[i] * pcm[i];
    }
    buckets.push(Math.sqrt(sum / (end - start)));
  }
  return buckets;
}

function formatTime(seconds: number): string {
  const m = Math.floor(seconds / 60);
  const s = Math.floor(seconds % 60);
  return `${m}:${s.toString().padStart(2, "0")}`;
}

export default function AudioWaveform({
  filePath,
  width = 600,
  height = 120,
}: AudioWaveformProps) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const [audioBuffer, setAudioBuffer] = useState<AudioBuffer | null>(null);
  const [isPlaceholder, setIsPlaceholder] = useState(false);
  const [waveformBuckets, setWaveformBuckets] = useState<number[]>([]);
  const [playing, setPlaying] = useState(false);
  const [currentTime, setCurrentTime] = useState(0);
  const [duration, setDuration] = useState(0);
  const audioCtxRef = useRef<AudioContext | null>(null);
  const sourceRef = useRef<AudioBufferSourceNode | null>(null);
  const startTimeRef = useRef(0);
  const startOffsetRef = useRef(0);
  const animFrameRef = useRef<number | null>(null);

  const NUM_BUCKETS = Math.max(60, Math.floor(width / 4));

  // Load audio file
  useEffect(() => {
    let cancelled = false;

    const load = async () => {
      try {
        // Backend returns base64-encoded binary
        const base64 = await invoke<string>("read_file_binary", { path: filePath });
        if (cancelled) return;

        // Decode base64
        const binary = atob(base64);
        const bytes = new Uint8Array(binary.length);
        for (let i = 0; i < binary.length; i++) {
          bytes[i] = binary.charCodeAt(i);
        }

        // Try to decode with Web Audio API
        if (!audioCtxRef.current) {
          audioCtxRef.current = new AudioContext();
        }

        try {
          const buf = await audioCtxRef.current.decodeAudioData(bytes.buffer.slice(0));
          if (cancelled) return;
          setAudioBuffer(buf);
          setDuration(buf.duration);
          const channel = buf.getChannelData(0);
          const buckets = buildWaveformBuckets(channel, NUM_BUCKETS);
          setWaveformBuckets(buckets);
          setIsPlaceholder(false);
        } catch {
          // Web Audio API can't decode — try manual WAV parse
          const pcm = parseWav(bytes);
          if (pcm && !cancelled) {
            setWaveformBuckets(buildWaveformBuckets(pcm, NUM_BUCKETS));
            setIsPlaceholder(false);
          } else if (!cancelled) {
            setWaveformBuckets(buildPlaceholderWaveform(NUM_BUCKETS));
            setIsPlaceholder(true);
          }
        }
      } catch {
        if (!cancelled) {
          setWaveformBuckets(buildPlaceholderWaveform(NUM_BUCKETS));
          setIsPlaceholder(true);
        }
      }
    };

    load();
    return () => { cancelled = true; };
  }, [filePath, NUM_BUCKETS]);

  // Draw waveform
  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas || waveformBuckets.length === 0) return;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;

    const dpr = window.devicePixelRatio || 1;
    const w = canvas.width / dpr;
    const h = canvas.height / dpr;

    ctx.clearRect(0, 0, canvas.width, canvas.height);

    const barWidth = w / waveformBuckets.length;
    const maxAmp = Math.max(...waveformBuckets, 0.001);
    const progressRatio = duration > 0 ? currentTime / duration : 0;

    for (let i = 0; i < waveformBuckets.length; i++) {
      const amp = waveformBuckets[i] / maxAmp;
      const barHeight = Math.max(2, amp * (h * 0.85));
      const x = i * barWidth;
      const y = (h - barHeight) / 2;
      const isPlayed = i / waveformBuckets.length <= progressRatio;
      ctx.fillStyle = isPlaceholder
        ? isPlayed ? "#6c7086" : "#313244"
        : isPlayed ? "#89b4fa" : "#45475a";
      ctx.fillRect(x + 1, y, Math.max(1, barWidth - 2), barHeight);
    }
  }, [waveformBuckets, currentTime, duration, isPlaceholder]);

  // Playback controls
  const handlePlay = useCallback(async () => {
    if (!audioBuffer) return;
    if (!audioCtxRef.current) audioCtxRef.current = new AudioContext();

    if (audioCtxRef.current.state === "suspended") {
      await audioCtxRef.current.resume();
    }

    if (sourceRef.current) {
      sourceRef.current.stop();
      sourceRef.current = null;
    }

    const source = audioCtxRef.current.createBufferSource();
    source.buffer = audioBuffer;
    source.connect(audioCtxRef.current.destination);
    source.start(0, startOffsetRef.current);
    source.onended = () => {
      setPlaying(false);
      startOffsetRef.current = 0;
      setCurrentTime(0);
    };
    sourceRef.current = source;
    startTimeRef.current = audioCtxRef.current.currentTime - startOffsetRef.current;
    setPlaying(true);

    const updateTime = () => {
      if (!audioCtxRef.current) return;
      const t = audioCtxRef.current.currentTime - startTimeRef.current;
      setCurrentTime(Math.min(t, audioBuffer.duration));
      if (t < audioBuffer.duration) {
        animFrameRef.current = requestAnimationFrame(updateTime);
      }
    };
    animFrameRef.current = requestAnimationFrame(updateTime);
  }, [audioBuffer]);

  const handlePause = useCallback(() => {
    if (sourceRef.current && audioCtxRef.current) {
      startOffsetRef.current = audioCtxRef.current.currentTime - startTimeRef.current;
      sourceRef.current.stop();
      sourceRef.current = null;
    }
    if (animFrameRef.current) cancelAnimationFrame(animFrameRef.current);
    setPlaying(false);
  }, []);

  const handleStop = useCallback(() => {
    if (sourceRef.current) {
      sourceRef.current.stop();
      sourceRef.current = null;
    }
    if (animFrameRef.current) cancelAnimationFrame(animFrameRef.current);
    startOffsetRef.current = 0;
    setPlaying(false);
    setCurrentTime(0);
  }, []);

  // Cleanup on unmount
  useEffect(() => {
    return () => {
      if (sourceRef.current) { try { sourceRef.current.stop(); } catch { /* ignore */ } }
      if (animFrameRef.current) cancelAnimationFrame(animFrameRef.current);
      audioCtxRef.current?.close();
    };
  }, []);

  const dpr = window.devicePixelRatio || 1;
  const fileName = filePath.split("/").pop() ?? filePath;

  return (
    <div style={{
      display: "flex",
      flexDirection: "column",
      gap: 8,
      padding: 12,
      background: "#1e1e2e",
      color: "#cdd6f4",
      fontFamily: "'JetBrains Mono', 'Fira Code', monospace",
      fontSize: 12,
    }}>
      <div style={{ color: "#89b4fa", fontWeight: 700 }}>{fileName}</div>

      {isPlaceholder && (
        <div style={{ fontSize: 10, color: "#6c7086" }}>Preview unavailable — showing placeholder waveform</div>
      )}

      <canvas
        ref={canvasRef}
        width={width * dpr}
        height={height * dpr}
        style={{ width, height, borderRadius: 6, background: "#181825", cursor: "pointer" }}
      />

      {/* Time display */}
      <div style={{ display: "flex", justifyContent: "space-between", fontSize: 11, color: "#6c7086" }}>
        <span>{formatTime(currentTime)}</span>
        {duration > 0 && <span>{formatTime(duration)}</span>}
      </div>

      {/* Controls */}
      <div style={{ display: "flex", gap: 6 }}>
        {!playing ? (
          <button
            onClick={handlePlay}
            disabled={!audioBuffer}
            style={ctrlBtnStyle(!audioBuffer)}
            title="Play"
          >▶ Play</button>
        ) : (
          <button onClick={handlePause} style={ctrlBtnStyle(false)} title="Pause">⏸ Pause</button>
        )}
        <button onClick={handleStop} disabled={!playing && currentTime === 0} style={ctrlBtnStyle(!playing && currentTime === 0)} title="Stop">■ Stop</button>
      </div>
    </div>
  );
}

function ctrlBtnStyle(disabled: boolean): React.CSSProperties {
  return {
    background: disabled ? "#313244" : "#89b4fa",
    color: disabled ? "#6c7086" : "#1e1e2e",
    border: "none",
    borderRadius: 4,
    padding: "4px 10px",
    cursor: disabled ? "not-allowed" : "pointer",
    fontSize: 12,
    fontFamily: "'JetBrains Mono', 'Fira Code', monospace",
  };
}
