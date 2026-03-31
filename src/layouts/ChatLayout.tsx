import { FerrumChat } from "../components/FerrumChat";

// TODO: extract to layout — chat mode switching (AiChat/FerrumChat/AiCli) is tangled with App.tsx state

interface ChatLayoutProps {
  visible: boolean;
  rootPath: string | null;
  activeFileContent?: string;
  activeFileName?: string;
  isFullscreen: boolean;
  onToggleFullscreen: () => void;
  sessionId?: string | null;
}

export function ChatLayout({
  visible,
  rootPath,
  activeFileContent,
  activeFileName,
  isFullscreen,
  onToggleFullscreen,
}: ChatLayoutProps) {
  return (
    <FerrumChat
      visible={visible}
      rootPath={rootPath ?? ""}
      activeFileContent={activeFileContent}
      activeFileName={activeFileName}
      isFullscreen={isFullscreen}
      onToggleFullscreen={onToggleFullscreen}
    />
  );
}
