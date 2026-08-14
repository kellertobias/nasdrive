import { create } from "zustand";
import { createJSONStorage, persist } from "zustand/middleware";
import { useShallow } from "zustand/react/shallow";

export type ViewMode = "grid" | "list" | "columns";
export type SortField = "name" | "size" | "modified_at";
export type SortDirection = "asc" | "desc";

interface ViewState {
  viewMode: ViewMode;
  sortField: SortField;
  sortDirection: SortDirection;
  selectedPaths: Set<string>;
  sidebarOpen: boolean;
  sidebarWidth: number;
  shareColumnWidth: number;
  folderColumnWidth: number;
  infoColumnWidth: number;

  setViewMode: (mode: ViewMode) => void;
  setSortField: (field: SortField) => void;
  toggleSortDirection: () => void;
  setSidebarWidth: (width: number) => void;
  setShareColumnWidth: (width: number) => void;
  setFolderColumnWidth: (width: number) => void;
  setInfoColumnWidth: (width: number) => void;
  select: (path: string) => void;
  toggleSelect: (path: string) => void;
  rangeSelect: (paths: string[]) => void;
  selectAll: (paths: string[]) => void;
  clearSelection: () => void;
  toggleSidebar: () => void;
}

/**
 * `localStorage.setItem` is synchronous main-thread work, and pane resizing
 * writes a width on every pointer move. Coalesce the writes onto a trailing
 * timer so a drag costs one write instead of one per frame; `pagehide` flushes
 * so a width is never lost when the tab goes away mid-drag.
 */
const PERSIST_DEBOUNCE_MS = 250;

function debouncedLocalStorage(): Storage {
  let timer: number | undefined;
  const pending = new Map<string, string>();

  const flush = () => {
    if (timer !== undefined) {
      window.clearTimeout(timer);
      timer = undefined;
    }
    for (const [key, value] of pending) {
      try {
        window.localStorage.setItem(key, value);
      } catch {
        // Quota or private-mode failures must not break the UI.
      }
    }
    pending.clear();
  };

  window.addEventListener("pagehide", flush);

  return {
    getItem: (key) => window.localStorage.getItem(key),
    removeItem: (key) => {
      pending.delete(key);
      window.localStorage.removeItem(key);
    },
    setItem: (key, value) => {
      pending.set(key, value);
      if (timer !== undefined) window.clearTimeout(timer);
      timer = window.setTimeout(flush, PERSIST_DEBOUNCE_MS);
    },
  } as Storage;
}

/** Widths a double-click on the matching resize handle restores. */
export const DEFAULT_SIDEBAR_WIDTH = 240;
export const DEFAULT_SHARE_COLUMN_WIDTH = 240;
export const DEFAULT_FOLDER_COLUMN_WIDTH = 280;
export const DEFAULT_INFO_COLUMN_WIDTH = 320;

export const useViewStore = create<ViewState>()(
  persist(
    (set, get) => ({
      viewMode: "grid",
      sortField: "name",
      sortDirection: "asc",
      selectedPaths: new Set<string>(),
      sidebarOpen: true,
      sidebarWidth: DEFAULT_SIDEBAR_WIDTH,
      shareColumnWidth: DEFAULT_SHARE_COLUMN_WIDTH,
      folderColumnWidth: DEFAULT_FOLDER_COLUMN_WIDTH,
      infoColumnWidth: DEFAULT_INFO_COLUMN_WIDTH,

      setViewMode: (mode) => set({ viewMode: mode }),
      setSortField: (field) => {
        const current = get();
        if (current.sortField === field) {
          set({
            sortDirection: current.sortDirection === "asc" ? "desc" : "asc",
          });
        } else {
          set({ sortField: field, sortDirection: "asc" });
        }
      },
      toggleSortDirection: () =>
        set((s) => ({
          sortDirection: s.sortDirection === "asc" ? "desc" : "asc",
        })),
      setSidebarWidth: (width) => set({ sidebarWidth: width }),
      setShareColumnWidth: (width) => set({ shareColumnWidth: width }),
      setFolderColumnWidth: (width) => set({ folderColumnWidth: width }),
      setInfoColumnWidth: (width) => set({ infoColumnWidth: width }),

      select: (path) => set({ selectedPaths: new Set([path]) }),
      toggleSelect: (path) =>
        set((s) => {
          const next = new Set(s.selectedPaths);
          if (next.has(path)) next.delete(path);
          else next.add(path);
          return { selectedPaths: next };
        }),
      rangeSelect: (paths) =>
        set((s) => {
          const next = new Set(s.selectedPaths);
          paths.forEach((p) => next.add(p));
          return { selectedPaths: next };
        }),
      selectAll: (paths) => set({ selectedPaths: new Set(paths) }),
      clearSelection: () => set({ selectedPaths: new Set() }),
      toggleSidebar: () => set((s) => ({ sidebarOpen: !s.sidebarOpen })),
    }),
    {
      name: "nasfiles-view",
      storage: createJSONStorage(debouncedLocalStorage),
      partialize: (state) => ({
        viewMode: state.viewMode,
        sortField: state.sortField,
        sortDirection: state.sortDirection,
        sidebarOpen: state.sidebarOpen,
        sidebarWidth: state.sidebarWidth,
        shareColumnWidth: state.shareColumnWidth,
        folderColumnWidth: state.folderColumnWidth,
        infoColumnWidth: state.infoColumnWidth,
      }),
    },
  ),
);

/**
 * Subscribe to a slice of the view store.
 *
 * Prefer this over calling `useViewStore()` with no argument. The bare call
 * subscribes the component to *every* field, so unrelated changes — most
 * painfully a pane width ticking on each pointer move during a resize drag —
 * re-render the file grid, the file list and the column browser along with it.
 * The shallow compare means a component only re-renders when a field it
 * actually named changes.
 */
export function useViewSlice<T>(selector: (state: ViewState) => T): T {
  return useViewStore(useShallow(selector));
}
