import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { invoke } from "@tauri-apps/api/core";
import RemoteSettings from "./RemoteSettings";

const mockInvoke = vi.mocked(invoke);

describe("RemoteSettings", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    // Default mock responses
    mockInvoke.mockImplementation(async (cmd: string) => {
      switch (cmd) {
        case "remote_get_info":
          return { running: false, port: 9876, local_ip: "192.168.1.100", connected_clients: [] };
        case "remote_list_devices":
          return [];
        case "remote_check_cert_expiry":
          return 350;
        case "remote_detect_network":
          return { local_ip: "192.168.1.100", tailscale_ip: null, tailscale_hostname: null, wireguard_ip: null };
        default:
          return null;
      }
    });
  });

  it("renders nothing when not visible", () => {
    const { container } = render(<RemoteSettings visible={false} />);
    expect(container.firstChild).toBeNull();
  });

  it("renders the remote access header", () => {
    render(<RemoteSettings visible={true} />);
    expect(screen.getByText("REMOTE ACCESS")).toBeInTheDocument();
  });

  it("shows server status as stopped initially", async () => {
    render(<RemoteSettings visible={true} />);
    await waitFor(() => {
      expect(screen.getByText("Stopped")).toBeInTheDocument();
    });
  });

  it("shows port input when server is stopped", async () => {
    render(<RemoteSettings visible={true} />);
    await waitFor(() => {
      expect(screen.getByText("Port:")).toBeInTheDocument();
    });
  });

  it("shows Start Server button when stopped", async () => {
    render(<RemoteSettings visible={true} />);
    await waitFor(() => {
      expect(screen.getByText("Start Server")).toBeInTheDocument();
    });
  });

  it("shows Regenerate Certificate button", async () => {
    render(<RemoteSettings visible={true} />);
    await waitFor(() => {
      expect(screen.getByText("Regenerate Certificate")).toBeInTheDocument();
    });
  });

  it("calls remote_start_server when Start Server is clicked", async () => {
    render(<RemoteSettings visible={true} />);

    await waitFor(() => {
      expect(screen.getByText("Start Server")).toBeInTheDocument();
    });

    fireEvent.click(screen.getByText("Start Server"));

    await waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith("remote_start_server", { port: 9876 });
    });
  });

  it("shows running server info", async () => {
    mockInvoke.mockImplementation(async (cmd: string) => {
      switch (cmd) {
        case "remote_get_info":
          return { running: true, port: 9876, local_ip: "192.168.1.100", connected_clients: [] };
        case "remote_list_devices":
          return [];
        case "remote_check_cert_expiry":
          return 350;
        case "remote_detect_network":
          return { local_ip: "192.168.1.100", tailscale_ip: null, tailscale_hostname: null, wireguard_ip: null };
        default:
          return null;
      }
    });

    render(<RemoteSettings visible={true} />);
    await waitFor(() => {
      expect(screen.getByText("Running")).toBeInTheDocument();
      expect(screen.getAllByText("192.168.1.100:9876").length).toBeGreaterThanOrEqual(1);
      expect(screen.getByText("Stop Server")).toBeInTheDocument();
    });
  });

  it("displays cert expiry info", async () => {
    render(<RemoteSettings visible={true} />);
    await waitFor(() => {
      expect(screen.getByText(/Certificate expires in 350 days/)).toBeInTheDocument();
    });
  });

  it("shows cert expiry warning when close to expiration", async () => {
    mockInvoke.mockImplementation(async (cmd: string) => {
      switch (cmd) {
        case "remote_get_info":
          return { running: false, port: 9876, local_ip: "192.168.1.100", connected_clients: [] };
        case "remote_list_devices":
          return [];
        case "remote_check_cert_expiry":
          return 15;
        case "remote_detect_network":
          return { local_ip: "192.168.1.100", tailscale_ip: null, tailscale_hostname: null, wireguard_ip: null };
        default:
          return null;
      }
    });

    render(<RemoteSettings visible={true} />);
    await waitFor(() => {
      expect(screen.getByText("(expiring soon!)")).toBeInTheDocument();
    });
  });

  it("shows network info with Tailscale when detected", async () => {
    mockInvoke.mockImplementation(async (cmd: string) => {
      switch (cmd) {
        case "remote_get_info":
          return { running: false, port: 9876, local_ip: "192.168.1.100", connected_clients: [] };
        case "remote_list_devices":
          return [];
        case "remote_check_cert_expiry":
          return 350;
        case "remote_detect_network":
          return { local_ip: "192.168.1.100", tailscale_ip: "100.64.0.5", tailscale_hostname: "myhost.tail.ts.net", wireguard_ip: null };
        default:
          return null;
      }
    });

    render(<RemoteSettings visible={true} />);
    await waitFor(() => {
      expect(screen.getByText("Tailscale IP:")).toBeInTheDocument();
      expect(screen.getByText("100.64.0.5")).toBeInTheDocument();
      expect(screen.getByText("Tailscale DNS:")).toBeInTheDocument();
    });
  });

  it("shows paired devices", async () => {
    mockInvoke.mockImplementation(async (cmd: string) => {
      switch (cmd) {
        case "remote_get_info":
          return { running: false, port: 9876, local_ip: "192.168.1.100", connected_clients: [] };
        case "remote_list_devices":
          return [{ id: "dev1", name: "iPhone", fingerprint: "abc123def456ghi789jkl012mno345pq", paired_at: "2026-01-01" }];
        case "remote_check_cert_expiry":
          return 350;
        case "remote_detect_network":
          return { local_ip: "192.168.1.100", tailscale_ip: null, tailscale_hostname: null, wireguard_ip: null };
        default:
          return null;
      }
    });

    render(<RemoteSettings visible={true} />);
    await waitFor(() => {
      expect(screen.getByText("iPhone")).toBeInTheDocument();
      expect(screen.getByText("Paired Devices (1)")).toBeInTheDocument();
    });
  });

  it("shows no devices paired message when empty", async () => {
    render(<RemoteSettings visible={true} />);
    await waitFor(() => {
      expect(screen.getByText("No devices paired yet.")).toBeInTheDocument();
    });
  });

  it("shows QR code button disabled when server is stopped", async () => {
    render(<RemoteSettings visible={true} />);
    await waitFor(() => {
      const qrBtn = screen.getByText("Show QR Code");
      expect(qrBtn).toBeDisabled();
    });
  });
});
