import { useState, useCallback, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";


interface ContentSearchResult {
  file: string;
  line: number;
  column: number;
  text: string;
  match_text: string;
}

interface SearchPanelProps {
  visible: boolean;
  rootPath: string;
  onFileOpen: (path: string, name: string) => void;
}

export default function SearchPanel({ visible, rootPath, onFileOpen }: SearchPanelProps) {
  const [searchQuery, setSearchQuery] = useState("");
  const [replaceQuery, setReplaceQuery] = useState("");
  const [results, setResults] = useState<ContentSearchResult[]>([]);
  const [searching, setSearching] = useState(false);
  const [searchError, setSearchError] = useState<string | null>(null);
  const [showReplace, setShowReplace] = useState(false);
  const [extensionFilter, setExtensionFilter] = useState("");
  const debounceRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const searchSeqRef = useRef(0);

  const doSearch = useCallback(async (query: string) => {
    if (!query.trim() || !rootPath) { setResults([]); setSearchError(null); return; }
    const seq = ++searchSeqRef.current;
    setSearching(true);
    setSearchError(null);
    try {
      const r = await invoke<ContentSearchResult[]>("search_in_files", {
        root: rootPath,
        pattern: query,
        extensions: extensionFilter || null,
      });
      if (searchSeqRef.current !== seq) return; // discard stale results
      setResults(r);
    } catch (err) {
      if (searchSeqRef.current === seq) {
        setResults([]);
        setSearchError(String(err));
      }
    }
    if (searchSeqRef.current === seq) setSearching(false);
  }, [rootPath, extensionFilter]);

  const handleSearchChange = (value: string) => {
    setSearchQuery(value);
    if (debounceRef.current) clearTimeout(debounceRef.current);
    debounceRef.current = setTimeout(() => doSearch(value), 400);
  };

  const handleReplace = async () => {
    if (!searchQuery || !replaceQuery) return;
    const filePaths = [...new Set(results.map(r => r.file))];
    try {
      const count = await invoke<number>("replace_in_files", {
        root: rootPath, search: searchQuery, replace: replaceQuery, filePaths
      });
      alert(`Replaced ${count} occurrences`);
      doSearch(searchQuery); // refresh
    } catch (err) {
      console.error("Replace failed:", err);
    }
  };

  // Group results by file
  const grouped = results.reduce<Record<string, ContentSearchResult[]>>((acc, r) => {
    (acc[r.file] ||= []).push(r);
    return acc;
  }, {});

  if (!visible) return null;

  return (
    <div className="search-panel">
      <div className="search-header">
        <span className="search-title">SEARCH</span>
        <button className="search-toggle-btn" onClick={() => setShowReplace(!showReplace)} title="Toggle Replace">
          {showReplace ? "\u25BC" : "\u25B6"}
        </button>
      </div>
      <div className="search-inputs">
        <input className="search-input" value={searchQuery} onChange={e => handleSearchChange(e.target.value)} placeholder="Search..." />
        {showReplace && (
          <div className="search-replace-row">
            <input className="search-input" value={replaceQuery} onChange={e => setReplaceQuery(e.target.value)} placeholder="Replace..." />
            <button className="search-replace-btn" onClick={handleReplace} disabled={!searchQuery || !replaceQuery} title="Replace All">
              All
            </button>
          </div>
        )}
        <input className="search-ext-input" value={extensionFilter} onChange={e => setExtensionFilter(e.target.value)} placeholder="File extensions (e.g. ts,rs)" />
      </div>
      <div className="search-results">
        {searching && <div className="search-status">Searching...</div>}
        {searchError && <div className="search-status" style={{ color: "#f85149" }}>Search error: {searchError}</div>}
        {!searching && !searchError && results.length === 0 && searchQuery && <div className="search-status">No results</div>}
        {Object.entries(grouped).map(([file, items]) => {
          const fileName = file.split("/").pop() || file;
          return (
            <div key={file} className="search-file-group">
              <div className="search-file-header" title={file}>
                <span className="search-file-name">{fileName}</span>
                <span className="search-file-count">{items.length}</span>
              </div>
              {items.map((item, i) => (
                <div key={i} className="search-result-item" onClick={() => onFileOpen(file, fileName)}>
                  <span className="search-result-line">L{item.line}</span>
                  <span className="search-result-text">{item.text.trim()}</span>
                </div>
              ))}
            </div>
          );
        })}
      </div>
    </div>
  );
}
