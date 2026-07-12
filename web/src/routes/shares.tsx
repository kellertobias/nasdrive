import { useQuery } from "@tanstack/react-query";
import { createFileRoute } from "@tanstack/react-router";
import { useEffect, useRef, useState } from "react";
import api from "../api/client";
import { Icon } from "../components/Icon";
import { TopBar } from "../components/TopBar";
import { SharesTab } from "./admin";

export const Route = createFileRoute("/shares")({ component: MySharesPage });

function MySharesPage() {
  const contentRef = useRef<HTMLDivElement>(null);
  const [filter, setFilter] = useState("all");
  const { data: user } = useQuery({ queryKey: ["me"], queryFn: api.me, retry: false });

  useEffect(() => {
    const root = contentRef.current;
    if (!root) return;
    const labelTables = () => root.querySelectorAll("table").forEach((table) => {
      table.classList.add("admin-responsive-table");
      const labels = Array.from(table.querySelectorAll("thead th")).map((cell) => cell.textContent?.trim() || "Actions");
      table.querySelectorAll("tbody tr").forEach((row) => Array.from(row.children).forEach((cell, index) => {
        if (cell instanceof HTMLElement) cell.dataset.label = labels[index] || "";
      }));
    });
    labelTables();
    const observer = new MutationObserver(labelTables);
    observer.observe(root, { childList: true, subtree: true });
    return () => observer.disconnect();
  }, []);

  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100vh", background: "var(--color-bg)" }}>
      <TopBar user={user ?? null} currentRoot="" />
      <main ref={contentRef} className="admin-content" style={{ padding: "var(--space-6)", flex: 1, overflow: "auto" }}>
        <div style={{ maxWidth: 1100, margin: "0 auto" }}>
          <div style={{ display: "flex", alignItems: "center", justifyContent: "space-between", marginBottom: "var(--space-6)" }}>
            <h1 style={{ fontSize: "var(--text-xl)", fontWeight: 600, margin: 0 }}>My shares</h1>
            <a href="/" style={{ display: "flex", alignItems: "center", gap: "var(--space-1)", color: "var(--color-fg-muted)", textDecoration: "none", fontSize: "var(--text-sm)" }}>
              <Icon name="arrowLeft" size={16} /> Back to Files
            </a>
          </div>
          <SharesTab filter={filter} setFilter={setFilter} selectedShareId={null} setSelectedShareId={() => {}} admin={false} />
        </div>
      </main>
    </div>
  );
}
