import { create } from "zustand";
import { persist } from "zustand/middleware";

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

export const useViewStore = create<ViewState>()(
  persist(
    (set, get) => ({
      viewMode: "grid",
      sortField: "name",
      sortDirection: "asc",
      selectedPaths: new Set<string>(),
      sidebarOpen: true,
      sidebarWidth: 240,
      shareColumnWidth: 240,
      folderColumnWidth: 280,
      infoColumnWidth: 320,

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
