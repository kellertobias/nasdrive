import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { createFileRoute } from "@tanstack/react-router";
import { useEffect, useState } from "react";
import api, {
	type AdminUserDetails,
	type FolderCaps,
	type IpBlocklistEntry,
	type PasskeyInfo,
	type Root,
	type SftpAccessLogEntry,
	type SftpTempUser,
	type TrustedDevice,
} from "../api/client";
import { Icon } from "../components/Icon";
import { TopBar } from "../components/TopBar";

export const Route = createFileRoute("/admin")({
	component: AdminDashboard,
});

interface AdminShare {
	id: string;
	owner_name: string;
	root_key: string;
	relative_path: string;
	is_directory: boolean;
	target_kind: string;
	has_password: boolean;
	allow_upload: boolean;
	allow_download: boolean;
	expires_at: number | null;
	created_at: number;
	revoked_at: number | null;
	access_count: number;
	last_accessed_at: number | null;
}

interface AccessLogEntry {
	id: string;
	share_id: string;
	occurred_at: number;
	ip: string | null;
	user_agent: string | null;
	action: string;
	path: string | null;
}

type Tab =
	| "shares"
	| "access-log"
	| "users"
	| "sftp"
	| "sftp-log"
	| "blocklist";

function AdminDashboard() {
	const [tab, setTab] = useState<Tab>("shares");
	const [shareFilter, setShareFilter] = useState("all");

	const { data: user } = useQuery({
		queryKey: ["me"],
		queryFn: api.me,
		retry: false,
		staleTime: 5 * 60 * 1000,
	});

	if (user && !user.is_admin) {
		return (
			<div
				style={{ display: "flex", flexDirection: "column", height: "100vh" }}
			>
				<TopBar user={user} currentRoot="" />
				<div
					style={{
						flex: 1,
						display: "flex",
						alignItems: "center",
						justifyContent: "center",
						flexDirection: "column",
						gap: "var(--space-4)",
						color: "var(--color-fg-muted)",
					}}
				>
					<Icon name="alertTriangle" size={48} />
					<h2 style={{ margin: 0, fontSize: "var(--text-xl)" }}>
						Access Denied
					</h2>
					<p style={{ fontSize: "var(--text-sm)" }}>
						Admin privileges required.
					</p>
				</div>
			</div>
		);
	}

	const tabs: { key: Tab; label: string }[] = [
		{ key: "shares", label: "Shares" },
		{ key: "access-log", label: "Access Log" },
		{ key: "users", label: "Users" },
		{ key: "sftp", label: "SFTP Guests" },
		{ key: "sftp-log", label: "SFTP Log" },
		{ key: "blocklist", label: "IP Blocklist" },
	];

	return (
		<div
			style={{
				display: "flex",
				flexDirection: "column",
				height: "100vh",
				background: "var(--color-bg)",
			}}
		>
			<TopBar user={user ?? null} currentRoot="" />

			<div style={{ padding: "var(--space-6)", flex: 1, overflow: "auto" }}>
				<div style={{ maxWidth: 1100, margin: "0 auto" }}>
					<div
						style={{
							display: "flex",
							alignItems: "center",
							justifyContent: "space-between",
							marginBottom: "var(--space-4)",
						}}
					>
						<h1
							style={{ fontSize: "var(--text-xl)", fontWeight: 600, margin: 0 }}
						>
							Administration
						</h1>
						<button
							type="button"
							onClick={() => {
								window.location.href = "/";
							}}
							style={{
								display: "flex",
								alignItems: "center",
								gap: "var(--space-1)",
								padding: "var(--space-2) var(--space-3)",
								border: "1px solid var(--color-border)",
								borderRadius: "var(--radius-md)",
								background: "transparent",
								color: "var(--color-fg-muted)",
								cursor: "pointer",
								fontSize: "var(--text-sm)",
								fontWeight: 500,
								transition: "all var(--duration-fast) var(--ease-out)",
							}}
							onMouseEnter={(e) => {
								(e.currentTarget as HTMLButtonElement).style.background =
									"var(--color-bg-muted)";
								(e.currentTarget as HTMLButtonElement).style.color =
									"var(--color-fg)";
							}}
							onMouseLeave={(e) => {
								(e.currentTarget as HTMLButtonElement).style.background =
									"transparent";
								(e.currentTarget as HTMLButtonElement).style.color =
									"var(--color-fg-muted)";
							}}
						>
							<Icon name="arrowLeft" size={16} />
							Back to Files
						</button>
					</div>

					{/* Tab bar */}
					<div
						style={{
							display: "flex",
							gap: "var(--space-1)",
							marginBottom: "var(--space-6)",
							borderBottom: "1px solid var(--color-border)",
						}}
					>
						{tabs.map((t) => (
							<button
								type="button"
								key={t.key}
								onClick={() => setTab(t.key)}
								style={{
									padding: "var(--space-2) var(--space-4)",
									border: "none",
									borderBottom:
										tab === t.key
											? "2px solid var(--color-accent)"
											: "2px solid transparent",
									background: "transparent",
									color:
										tab === t.key ? "var(--color-fg)" : "var(--color-fg-muted)",
									cursor: "pointer",
									fontSize: "var(--text-sm)",
									fontWeight: 500,
									marginBottom: -1,
									transition: "all var(--duration-fast) var(--ease-out)",
								}}
							>
								{t.label}
							</button>
						))}
					</div>

					{tab === "shares" && (
						<SharesTab filter={shareFilter} setFilter={setShareFilter} />
					)}
					{tab === "access-log" && <AccessLogTab />}
					{tab === "users" && <UsersTab />}
					{tab === "sftp" && <SftpGuestsTab />}
					{tab === "sftp-log" && <SftpAccessLogTab />}
					{tab === "blocklist" && <BlocklistTab />}
				</div>
			</div>
		</div>
	);
}

function SftpAccessLogTab() {
	const { data, isLoading } = useQuery({
		queryKey: ["admin-sftp-access-log"],
		queryFn: api.listSftpAccessLog,
		staleTime: 10_000,
	});

	const entries: SftpAccessLogEntry[] = data?.entries ?? [];

	if (isLoading) return <LoadingRows />;
	if (entries.length === 0) return <EmptyMessage text="No SFTP access log entries" />;

	return (
		<table style={tableStyle}>
			<thead>
				<tr>
					<th style={thStyle}>Time</th>
					<th style={thStyle}>Principal</th>
					<th style={thStyle}>Action</th>
					<th style={thStyle}>Path</th>
					<th style={thStyle}>IP</th>
					<th style={thStyle}>Result</th>
					<th style={thStyle}>Error</th>
						<th style={thStyle} />
				</tr>
			</thead>
			<tbody>
				{entries.map((entry) => (
					<tr key={entry.id}>
						<td style={tdStyle}>{new Date(entry.occurred_at).toLocaleString()}</td>
						<td style={tdStyle}>
							<div style={{ display: "flex", flexDirection: "column", gap: 2 }}>
								<span>{entry.principal_kind}</span>
								<code style={{ fontSize: "var(--text-xs)", color: "var(--color-fg-muted)" }}>
									{entry.principal_id.slice(0, 12)}
								</code>
							</div>
						</td>
						<td style={tdStyle}>
							<span
								style={{
									padding: "1px 6px",
									borderRadius: "var(--radius-full)",
									background: "var(--color-bg-muted)",
									fontSize: "var(--text-xs)",
									fontWeight: 500,
								}}
							>
								{entry.action}
							</span>
						</td>
						<td style={tdStyle}>
							<code style={{ fontSize: "var(--text-xs)" }}>
								{entry.root_key ? `${entry.root_key}/${entry.path || ""}` : "—"}
							</code>
						</td>
						<td style={tdStyle}>{entry.ip || "—"}</td>
						<td style={{ ...tdStyle, color: entry.success ? "var(--color-success)" : "var(--color-danger)", fontWeight: 500 }}>
							{entry.success ? "OK" : "Failed"}
						</td>
						<td style={tdStyle}>{entry.error || "—"}</td>
						<td style={tdStyle}>
							{entry.ip ? (
								<BlockIpButton
									ip={entry.ip}
									reason={`SFTP ${entry.action} from ${entry.principal_kind}`}
								/>
							) : null}
						</td>
					</tr>
				))}
			</tbody>
		</table>
	);
}

// ---------------------------------------------------------------------------
// IP blocklist tab
// ---------------------------------------------------------------------------

function BlocklistTab() {
	const queryClient = useQueryClient();
	const [ip, setIp] = useState("");
	const [reason, setReason] = useState("");
	const [error, setError] = useState("");

	const { data, isLoading } = useQuery({
		queryKey: ["admin-ip-blocklist"],
		queryFn: api.listIpBlocklist,
		staleTime: 10_000,
	});

	const addMutation = useMutation({
		mutationFn: () =>
			api.addIpBlocklistEntry(ip.trim(), reason.trim() || undefined),
		onSuccess: () => {
			setIp("");
			setReason("");
			setError("");
			queryClient.invalidateQueries({ queryKey: ["admin-ip-blocklist"] });
		},
		onError: (err) =>
			setError(err instanceof Error ? err.message : String(err)),
	});

	const removeMutation = useMutation({
		mutationFn: api.removeIpBlocklistEntry,
		onSuccess: () =>
			queryClient.invalidateQueries({ queryKey: ["admin-ip-blocklist"] }),
	});

	const entries: IpBlocklistEntry[] = data?.entries ?? [];

	return (
		<div>
			<p
				style={{
					margin: "0 0 var(--space-4)",
					color: "var(--color-fg-muted)",
					fontSize: "var(--text-sm)",
				}}
			>
				Blocked IPs are denied SFTP authentication regardless of credentials.
				Addresses that attempt a root SFTP login are added here automatically.
				Remove an entry to lift the block.
			</p>
			<div
				style={{
					display: "flex",
					alignItems: "center",
					gap: "var(--space-2)",
					marginBottom: "var(--space-5)",
					flexWrap: "wrap",
				}}
			>
				<input
					value={ip}
					onChange={(e) => setIp(e.target.value)}
					onKeyDown={(e) => {
						if (e.key === "Enter" && ip.trim() && !addMutation.isPending) {
							addMutation.mutate();
						}
					}}
					placeholder="IP address (e.g. 203.0.113.5)"
					style={{ ...inputStyle, width: 220, fontFamily: "monospace" }}
				/>
				<input
					value={reason}
					onChange={(e) => setReason(e.target.value)}
					placeholder="Reason (optional)"
					style={{ ...inputStyle, width: 220 }}
				/>
				<button
					type="button"
					disabled={!ip.trim() || addMutation.isPending}
					onClick={() => addMutation.mutate()}
					style={{
						...actionButtonStyle,
						background: "var(--color-accent)",
						color: "var(--color-accent-fg)",
						borderColor: "var(--color-accent)",
					}}
				>
					<Icon name="alertTriangle" size={16} />
					Block IP
				</button>
				{error && (
					<span
						style={{
							color: "var(--color-danger)",
							fontSize: "var(--text-sm)",
						}}
					>
						{error}
					</span>
				)}
			</div>
			{isLoading ? (
				<LoadingRows />
			) : entries.length === 0 ? (
				<EmptyMessage text="No blocked IPs" />
			) : (
				<table style={tableStyle}>
					<thead>
						<tr>
							<th style={thStyle}>IP Address</th>
							<th style={thStyle}>Reason</th>
							<th style={thStyle}>Blocked</th>
							<th style={thStyle}>Last attempt</th>
							<th style={thStyle}>Attempts</th>
							<th style={thStyle} />
						</tr>
					</thead>
					<tbody>
						{entries.map((entry) => (
							<tr key={entry.ip}>
								<td style={tdStyle}>
									<code style={{ fontSize: "var(--text-xs)" }}>{entry.ip}</code>
								</td>
								<td style={tdStyle}>{entry.reason || "—"}</td>
								<td style={tdStyle}>
									{new Date(entry.blocked_at).toLocaleString()}
								</td>
								<td style={tdStyle}>
									{new Date(entry.last_seen_at).toLocaleString()}
								</td>
								<td style={tdStyle}>{entry.hit_count}</td>
								<td style={tdStyle}>
									<div
										style={{ display: "flex", justifyContent: "flex-end" }}
									>
										<button
											type="button"
											disabled={removeMutation.isPending}
											onClick={() => removeMutation.mutate(entry.ip)}
											style={{
												...actionButtonStyle,
												color: "var(--color-danger)",
											}}
										>
											<Icon name="x" size={16} />
											Remove
										</button>
									</div>
								</td>
							</tr>
						))}
					</tbody>
				</table>
			)}
		</div>
	);
}

/**
 * Inline "Block" action for a log row. Adds the given IP to the blocklist and
 * shows a "Blocked" confirmation state. Refreshes the IP Blocklist tab.
 */
function BlockIpButton({ ip, reason }: { ip: string; reason: string }) {
	const queryClient = useQueryClient();
	const [blocked, setBlocked] = useState(false);

	const mutation = useMutation({
		mutationFn: () => api.addIpBlocklistEntry(ip, reason),
		onSuccess: () => {
			setBlocked(true);
			queryClient.invalidateQueries({ queryKey: ["admin-ip-blocklist"] });
		},
	});

	return (
		<button
			type="button"
			disabled={blocked || mutation.isPending}
			onClick={() => mutation.mutate()}
			title={blocked ? `${ip} is blocked` : `Block ${ip}`}
			style={{
				...actionButtonStyle,
				padding: "var(--space-1) var(--space-2)",
				color: blocked ? "var(--color-fg-muted)" : "var(--color-danger)",
			}}
		>
			<Icon name={blocked ? "checkCircle" : "alertTriangle"} size={14} />
			{blocked ? "Blocked" : "Block"}
		</button>
	);
}

// ---------------------------------------------------------------------------
// SFTP guests tab
// ---------------------------------------------------------------------------

function SftpGuestsTab() {
	const queryClient = useQueryClient();
	const [displayName, setDisplayName] = useState("");
	const [rootKey, setRootKey] = useState("");
	const [path, setPath] = useState("");
	const [canWrite, setCanWrite] = useState(false);
	const [expiresIn, setExpiresIn] = useState(24 * 60 * 60);
	const [publicKey, setPublicKey] = useState("");
	const [error, setError] = useState("");

	const { data: me } = useQuery({
		queryKey: ["me"],
		queryFn: api.me,
		staleTime: 5 * 60 * 1000,
	});
	const { data, isLoading } = useQuery({
		queryKey: ["admin-sftp-temp-users"],
		queryFn: api.listSftpTempUsers,
		staleTime: 10_000,
	});

	const createMutation = useMutation({
		mutationFn: () => api.createSftpTempUser({
			display_name: displayName,
			root_key: rootKey || me?.roots[0]?.key || "",
			path,
			can_write: canWrite,
			expires_in: expiresIn,
			public_key: publicKey,
		}),
		onSuccess: () => {
			setDisplayName("");
			setPath("");
			setCanWrite(false);
			setPublicKey("");
			setError("");
			queryClient.invalidateQueries({ queryKey: ["admin-sftp-temp-users"] });
		},
		onError: (err) => setError(String(err)),
	});

	const extendMutation = useMutation({
		mutationFn: ({ id, expiresIn }: { id: string; expiresIn: number }) => api.extendSftpTempUser(id, expiresIn),
		onSuccess: () => queryClient.invalidateQueries({ queryKey: ["admin-sftp-temp-users"] }),
	});
	const revokeMutation = useMutation({
		mutationFn: api.revokeSftpTempUser,
		onSuccess: () => queryClient.invalidateQueries({ queryKey: ["admin-sftp-temp-users"] }),
	});

	const users: SftpTempUser[] = data?.users ?? [];
	const roots = me?.roots ?? [];
	const selectedRoot = rootKey || roots[0]?.key || "";
	const fillKeyForNewShare = (user: SftpTempUser) => {
		setDisplayName(user.display_name);
		setPublicKey(user.public_key || "");
		setError("");
	};

	return (
		<div>
			<div style={{ display: "grid", gridTemplateColumns: "minmax(160px, 1fr) minmax(140px, 180px) minmax(140px, 1fr)", gap: "var(--space-2)", marginBottom: "var(--space-3)" }}>
				<input value={displayName} onChange={(e) => setDisplayName(e.target.value)} placeholder="Guest name" style={inputStyle} />
				<select value={selectedRoot} onChange={(e) => setRootKey(e.target.value)} style={inputStyle}>
					{roots.filter((r) => r.kind === "common").map((root) => (
						<option key={root.key} value={root.key}>{root.display_name}</option>
					))}
				</select>
				<input value={path} onChange={(e) => setPath(e.target.value)} placeholder="Folder path" style={inputStyle} />
			</div>
			<textarea
				value={publicKey}
				onChange={(e) => setPublicKey(e.target.value)}
				placeholder="ssh-ed25519 AAAA..."
				rows={3}
				style={{ ...inputStyle, fontFamily: "monospace", resize: "vertical", marginBottom: "var(--space-3)" }}
			/>
			<div style={{ display: "flex", alignItems: "center", gap: "var(--space-3)", marginBottom: "var(--space-5)", flexWrap: "wrap" }}>
				<label style={{ display: "flex", alignItems: "center", gap: "var(--space-2)", fontSize: "var(--text-sm)" }}>
					<input type="checkbox" checked={canWrite} onChange={(e) => setCanWrite(e.target.checked)} />
					Read/write
				</label>
				<select value={expiresIn} onChange={(e) => setExpiresIn(Number(e.target.value))} style={{ ...inputStyle, width: 130 }}>
					{expiryOptions.map((opt) => <option key={opt.value} value={opt.value}>{opt.label}</option>)}
				</select>
				<button
					type="button"
					disabled={!displayName.trim() || !publicKey.trim() || !selectedRoot || createMutation.isPending}
					onClick={() => createMutation.mutate()}
					style={{ ...actionButtonStyle, background: "var(--color-accent)", color: "var(--color-accent-fg)", borderColor: "var(--color-accent)" }}
				>
					<Icon name="user" size={16} />
					Create guest
				</button>
				{error && <span style={{ color: "var(--color-danger)", fontSize: "var(--text-sm)" }}>{error}</span>}
			</div>

			{isLoading ? (
				<LoadingRows />
			) : users.length === 0 ? (
				<EmptyMessage text="No SFTP guests found" />
			) : (
				<table style={tableStyle}>
					<thead>
						<tr>
							<th style={thStyle}>Guest</th>
							<th style={thStyle}>Path</th>
							<th style={thStyle}>Permissions</th>
							<th style={thStyle}>Expires</th>
							<th style={thStyle}>Fingerprint</th>
							<th style={thStyle}>Status</th>
							<th style={thStyle}></th>
						</tr>
					</thead>
					<tbody>
						{users.map((u) => {
							const expired = u.expires_at <= Date.now();
							const revoked = !!u.revoked_at;
							const status = revoked ? "Revoked" : expired ? "Expired" : "Active";
							return (
								<tr key={u.id}>
									<td style={tdStyle}>{u.display_name}</td>
									<td style={tdStyle}><code style={{ fontSize: "var(--text-xs)" }}>{u.root_key}/{u.relative_path}</code></td>
									<td style={tdStyle}>{u.can_write ? "Read/write" : "Read-only"}</td>
									<td style={tdStyle}>{new Date(u.expires_at).toLocaleString()}</td>
									<td style={tdStyle}>
										<code style={{ fontSize: "var(--text-xs)" }}>{u.key_fingerprint || "—"}</code>
										{u.public_key && (
											<button
												type="button"
												onClick={() => fillKeyForNewShare(u)}
												style={{
													display: "block",
													marginTop: "var(--space-1)",
													padding: 0,
													border: 0,
													background: "transparent",
													color: "var(--color-accent)",
													fontSize: "var(--text-xs)",
													cursor: "pointer",
												}}
											>
												use key for new share
											</button>
										)}
									</td>
									<td style={{ ...tdStyle, color: status === "Active" ? "var(--color-success)" : "var(--color-fg-muted)", fontWeight: 500 }}>{status}</td>
									<td style={tdStyle}>
										<div style={{ display: "flex", gap: "var(--space-1)", justifyContent: "flex-end" }}>
											<select
												title={
													revoked
														? "Extending a revoked guest re-enables its key and restores access until the new expiry."
														: "Set a new expiry for this guest. Note: extending a revoked guest re-enables its key."
												}
												onChange={(e) => {
													if (e.target.value) {
														extendMutation.mutate({ id: u.id, expiresIn: Number(e.target.value) });
														e.currentTarget.value = "";
													}
												}}
												defaultValue=""
												style={{ ...inputStyle, width: 110, padding: "var(--space-1) var(--space-2)" }}
											>
												<option value="">Extend</option>
												{expiryOptions.map((opt) => <option key={opt.value} value={opt.value}>{opt.label}</option>)}
											</select>
											{!revoked && (
												<button type="button" onClick={() => revokeMutation.mutate(u.id)} style={{ ...actionButtonStyle, color: "var(--color-danger)" }}>
													<Icon name="x" size={16} />
													Revoke
												</button>
											)}
										</div>
									</td>
								</tr>
							);
						})}
					</tbody>
				</table>
			)}
		</div>
	);
}

// ---------------------------------------------------------------------------
// Shares tab
// ---------------------------------------------------------------------------

function SharesTab({
	filter,
	setFilter,
}: {
	filter: string;
	setFilter: (v: string) => void;
}) {
	const { data, isLoading } = useQuery({
		queryKey: ["admin-shares", filter],
		queryFn: () =>
			fetch(`/api/admin/shares?limit=200&status=${filter}`, {
				headers: { "X-NasFiles-Request": "1" },
			}).then((r) => r.json()),
		staleTime: 10_000,
	});

	const shares: AdminShare[] = data?.shares ?? [];

	return (
		<div>
			{/* Filter pills */}
			<div
				style={{
					display: "flex",
					gap: "var(--space-1)",
					marginBottom: "var(--space-4)",
				}}
			>
				{["all", "active", "expired", "revoked"].map((s) => (
					<button
						type="button"
						key={s}
						onClick={() => setFilter(s)}
						style={{
							padding: "var(--space-1) var(--space-3)",
							border: "1px solid",
							borderColor:
								filter === s ? "var(--color-accent)" : "var(--color-border)",
							borderRadius: "var(--radius-full)",
							background:
								filter === s ? "var(--color-accent-muted)" : "transparent",
							color:
								filter === s ? "var(--color-accent)" : "var(--color-fg-muted)",
							cursor: "pointer",
							fontSize: "var(--text-xs)",
							fontWeight: 500,
							textTransform: "capitalize",
						}}
					>
						{s}
					</button>
				))}
			</div>

			{isLoading ? (
				<LoadingRows />
			) : shares.length === 0 ? (
				<EmptyMessage text="No shares found" />
			) : (
				<table style={tableStyle}>
					<thead>
						<tr>
							<th style={thStyle}>Owner</th>
							<th style={thStyle}>Path</th>
							<th style={thStyle}>Type</th>
							<th style={thStyle}>Permissions</th>
							<th style={thStyle}>Created</th>
							<th style={thStyle}>Expires</th>
							<th style={thStyle}>Views</th>
							<th style={thStyle}>Last Access</th>
							<th style={thStyle}>Status</th>
						</tr>
					</thead>
					<tbody>
						{shares.map((s) => {
							const now = Date.now();
							const isExpired = s.expires_at ? s.expires_at <= now : false;
							const isRevoked = !!s.revoked_at;
							const status = isRevoked
								? "Revoked"
								: isExpired
									? "Expired"
									: "Active";
							const statusColor = isRevoked
								? "var(--color-danger)"
								: isExpired
									? "var(--color-fg-muted)"
									: "var(--color-success)";

							return (
								<tr key={s.id}>
									<td style={tdStyle}>{s.owner_name}</td>
									<td style={tdStyle}>
										<code style={{ fontSize: "var(--text-xs)" }}>
											{s.root_key}/{s.relative_path || ""}
										</code>
									</td>
									<td style={tdStyle}>
										<div style={{ display: "flex", gap: "var(--space-1)", flexWrap: "wrap" }}>
											<span
												style={{
													padding: "1px 6px",
													borderRadius: "var(--radius-full)",
													background: "var(--color-bg-muted)",
													fontSize: "var(--text-xs)",
												}}
											>
												{s.target_kind}
											</span>
											<span
												style={{
													padding: "1px 6px",
													borderRadius: "var(--radius-full)",
													background: "var(--color-bg-muted)",
													fontSize: "var(--text-xs)",
												}}
											>
												{s.has_password ? "Password" : "Public"}
											</span>
										</div>
									</td>
									<td style={tdStyle}>
										{[s.allow_download && "↓", s.allow_upload && "↑"]
											.filter(Boolean)
											.join(" ")}
									</td>
									<td style={tdStyle}>
										{new Date(s.created_at).toLocaleDateString()}
									</td>
									<td style={tdStyle}>
										{(() => {
											if (!s.expires_at) return "Never";
											if (isExpired) return "Expired";
											const diffMs = s.expires_at - now;
											const days = Math.ceil(diffMs / (1000 * 60 * 60 * 24));
											const d = new Date(s.expires_at);
											const pad = (n: number) => n.toString().padStart(2, "0");
											const exactTime = `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}`;
											return (
												<span
													title={exactTime}
													style={{
														cursor: "help",
														borderBottom: "1px dotted var(--color-fg-muted)",
													}}
												>
													in {days} day{days === 1 ? "" : "s"}
												</span>
											);
										})()}
									</td>
									<td style={tdStyle}>
										{s.access_count}
									</td>
									<td style={tdStyle}>
										{s.last_accessed_at ? new Date(s.last_accessed_at).toLocaleDateString() : 'Never'}
									</td>
									<td
										style={{ ...tdStyle, color: statusColor, fontWeight: 500 }}
									>
										{status}
									</td>
								</tr>
							);
						})}
					</tbody>
				</table>
			)}
		</div>
	);
}

// ---------------------------------------------------------------------------
// Access log tab
// ---------------------------------------------------------------------------

function AccessLogTab() {
	const { data, isLoading } = useQuery({
		queryKey: ["admin-access-log"],
		queryFn: () =>
			fetch("/api/admin/access-log?limit=200", {
				headers: { "X-NasFiles-Request": "1" },
			}).then((r) => r.json()),
		staleTime: 10_000,
	});

	const entries: AccessLogEntry[] = data?.entries ?? [];

	if (isLoading) return <LoadingRows />;
	if (entries.length === 0)
		return <EmptyMessage text="No access log entries" />;

	return (
		<table style={tableStyle}>
			<thead>
				<tr>
					<th style={thStyle}>Time</th>
					<th style={thStyle}>Action</th>
					<th style={thStyle}>Share</th>
					<th style={thStyle}>Path</th>
					<th style={thStyle}>IP</th>
					<th style={thStyle} />
				</tr>
			</thead>
			<tbody>
				{entries.map((e) => (
					<tr key={e.id}>
						<td style={tdStyle}>{new Date(e.occurred_at).toLocaleString()}</td>
						<td style={tdStyle}>
							<span
								style={{
									padding: "1px 6px",
									borderRadius: "var(--radius-full)",
									background:
										e.action === "auth_fail"
											? "var(--color-danger-muted)"
											: "var(--color-bg-muted)",
									color:
										e.action === "auth_fail"
											? "var(--color-danger)"
											: "var(--color-fg)",
									fontSize: "var(--text-xs)",
									fontWeight: 500,
								}}
							>
								{e.action}
							</span>
						</td>
						<td style={tdStyle}>
							<code style={{ fontSize: "var(--text-xs)" }}>
								{e.share_id.slice(0, 8)}…
							</code>
						</td>
						<td style={tdStyle}>{e.path || "—"}</td>
						<td style={tdStyle}>{e.ip || "—"}</td>
						<td style={tdStyle}>
							{e.ip ? (
								<BlockIpButton ip={e.ip} reason={`access log (${e.action})`} />
							) : null}
						</td>
					</tr>
				))}
			</tbody>
		</table>
	);
}

// ---------------------------------------------------------------------------
// Users tab
// ---------------------------------------------------------------------------

function UsersTab() {
	const queryClient = useQueryClient();
	const [username, setUsername] = useState("");
	const [displayName, setDisplayName] = useState("");
	const [isAdmin, setIsAdmin] = useState(false);
	const [hasHome, setHasHome] = useState(false);
	const [createPermissions, setCreatePermissions] = useState<
		Record<string, FolderCaps>
	>({});
	const [createdPassword, setCreatedPassword] = useState<{
		username: string;
		password: string;
	} | null>(null);
	const [resetPassword, setResetPassword] = useState<{
		username: string;
		password: string;
	} | null>(null);
	const [securityDetails, setSecurityDetails] = useState<{
		user: AdminUserDetails;
		passkeys: PasskeyInfo[];
		devices: TrustedDevice[];
	} | null>(null);
	const [editingUser, setEditingUser] = useState<AdminUserDetails | null>(null);
	const [securityLoadingUserId, setSecurityLoadingUserId] = useState("");
	const [securityError, setSecurityError] = useState("");

	const { data: me } = useQuery({
		queryKey: ["me"],
		queryFn: api.me,
		staleTime: 5 * 60 * 1000,
	});

	const { data, isLoading } = useQuery({
		queryKey: ["admin-users"],
		queryFn: api.listAdminUsers,
		staleTime: 30_000,
	});

	const localMode = me?.auth.mode === "local";
	const commonRoots = me?.roots.filter((root) => root.kind === "common") ?? [];
	const users: AdminUserDetails[] = data?.users ?? [];

	const createMutation = useMutation({
		mutationFn: () =>
			api.createLocalUser({
				username: username.trim(),
				display_name: displayName.trim() || undefined,
				is_admin: isAdmin,
				has_home: hasHome,
				folder_permissions: normalizePermissions(commonRoots, createPermissions),
			}),
		onSuccess: (result) => {
			setCreatedPassword({
				username: result.username,
				password: result.password,
			});
			setResetPassword(null);
			setUsername("");
			setDisplayName("");
			setIsAdmin(false);
			setHasHome(false);
			setCreatePermissions({});
			queryClient.invalidateQueries({ queryKey: ["admin-users"] });
		},
	});

	const resetMutation = useMutation({
		mutationFn: (user: AdminUserDetails) => api.resetLocalUserPassword(user.id),
		onSuccess: (result, user) => {
			setCreatedPassword(null);
			setResetPassword({ username: user.username, password: result.password });
			queryClient.invalidateQueries({ queryKey: ["admin-users"] });
		},
	});

	const loadSecurityDetails = async (user: AdminUserDetails) => {
		setSecurityError("");
		setSecurityLoadingUserId(user.id);
		try {
			const [passkeys, devices] = await Promise.all([
				api.listAdminPasskeys(user.id),
				api.listAdminTrustedDevices(user.id),
			]);
			setSecurityDetails({
				user,
				passkeys: passkeys.passkeys,
				devices: devices.devices,
			});
		} catch (err) {
			setSecurityError(String(err));
		} finally {
			setSecurityLoadingUserId("");
		}
	};

	const revokePasskey = async (user: AdminUserDetails, passkeyId: string) => {
		setSecurityError("");
		try {
			await api.revokeAdminPasskey(user.id, passkeyId);
			await loadSecurityDetails(user);
			queryClient.invalidateQueries({ queryKey: ["admin-users"] });
		} catch (err) {
			setSecurityError(String(err));
		}
	};

	const revokeTrustedDevice = async (user: AdminUserDetails, deviceId: string) => {
		setSecurityError("");
		try {
			await api.revokeAdminTrustedDevice(user.id, deviceId);
			await loadSecurityDetails(user);
			queryClient.invalidateQueries({ queryKey: ["admin-users"] });
		} catch (err) {
			setSecurityError(String(err));
		}
	};

	return (
		<div style={{ display: "grid", gap: "var(--space-5)" }}>
			{localMode && (
				<div style={panelStyle}>
					<div
						style={{
							display: "grid",
							gridTemplateColumns:
								"minmax(160px, 1fr) minmax(160px, 1fr) auto",
							gap: "var(--space-2)",
							marginBottom: "var(--space-3)",
							alignItems: "center",
						}}
					>
						<input
							value={username}
							onChange={(e) => setUsername(e.target.value)}
							placeholder="Username"
							style={inputStyle}
						/>
						<input
							value={displayName}
							onChange={(e) => setDisplayName(e.target.value)}
							placeholder="Display name"
							style={inputStyle}
						/>
						<button
							type="button"
							disabled={!username.trim() || createMutation.isPending}
							onClick={() => createMutation.mutate()}
							style={{
								...actionButtonStyle,
								background: "var(--color-accent)",
								borderColor: "var(--color-accent)",
								color: "var(--color-accent-fg)",
								whiteSpace: "nowrap",
							}}
						>
							<Icon name="user" size={16} />
							Create user
						</button>
					</div>
					<div
						style={{
							display: "flex",
							alignItems: "center",
							gap: "var(--space-3)",
							marginBottom: "var(--space-3)",
							flexWrap: "wrap",
						}}
					>
						<label style={checkboxLabelStyle}>
							<input
								type="checkbox"
								checked={isAdmin}
								onChange={(e) => setIsAdmin(e.target.checked)}
							/>
							Admin
						</label>
						<label style={checkboxLabelStyle}>
							<input
								type="checkbox"
								checked={hasHome}
								onChange={(e) => setHasHome(e.target.checked)}
							/>
							Home folder
						</label>
					</div>
					{username.trim() && (
						<PermissionMatrix
							roots={commonRoots}
							permissions={createPermissions}
							onChange={(rootKey, field, value) =>
								setCreatePermissions((current) =>
									withPermission(current, rootKey, field, value),
								)
							}
						/>
					)}
					{createMutation.error && (
						<div style={{ ...errorTextStyle, marginTop: "var(--space-3)" }}>
							{String(createMutation.error)}
						</div>
					)}
				</div>
			)}

			{createdPassword && (
				<PasswordResult
					label={`Created ${createdPassword.username}`}
					password={createdPassword.password}
				/>
			)}
			{resetPassword && (
				<PasswordResult
					label={`Reset ${resetPassword.username}`}
					password={resetPassword.password}
				/>
			)}

			{isLoading ? (
				<LoadingRows />
			) : users.length === 0 ? (
				<EmptyMessage text="No users found" />
			) : (
				<table style={tableStyle}>
					<thead>
						<tr>
							<th style={thStyle}>User</th>
							<th style={thStyle}>Mode</th>
							<th style={thStyle}>Access</th>
							<th style={thStyle}>Security</th>
							<th style={thStyle}>Activity</th>
							<th style={thStyle}></th>
						</tr>
					</thead>
					<tbody>
						{users.map((u) => (
							<AdminUserRow
								key={u.id}
								user={u}
								commonRoots={commonRoots}
								localMode={localMode}
								resetPending={resetMutation.isPending}
								onReset={(user) => resetMutation.mutate(user)}
								onOpenSecurity={(user) => loadSecurityDetails(user)}
								onEdit={(user) => setEditingUser(user)}
								securityLoading={securityLoadingUserId === u.id}
							/>
						))}
					</tbody>
				</table>
			)}

			{editingUser && (
				<UserEditModal
					user={editingUser}
					commonRoots={commonRoots}
					onClose={() => setEditingUser(null)}
				/>
			)}

			{securityDetails && (
				<Modal
					title={`${securityDetails.user.display_name} security`}
					onClose={() => setSecurityDetails(null)}
				>
					<div
						style={{
							display: "grid",
							gap: "var(--space-3)",
						}}
					>
						{securityError && <div style={errorTextStyle}>{securityError}</div>}
						<SecurityList
							title="Passkeys"
							empty="No passkeys"
							items={securityDetails.passkeys}
							onRevoke={(id) => revokePasskey(securityDetails.user, id)}
						/>
						<SecurityList
							title="Trusted TOTP devices"
							empty="No trusted devices"
							items={securityDetails.devices}
							onRevoke={(id) => revokeTrustedDevice(securityDetails.user, id)}
						/>
					</div>
				</Modal>
			)}
		</div>
	);
}

function AdminUserRow({
	user,
	commonRoots,
	localMode,
	resetPending,
	securityLoading,
	onReset,
	onOpenSecurity,
	onEdit,
}: {
	user: AdminUserDetails;
	commonRoots: Root[];
	localMode: boolean;
	resetPending: boolean;
	securityLoading: boolean;
	onReset: (user: AdminUserDetails) => void;
	onOpenSecurity: (user: AdminUserDetails) => void;
	onEdit: (user: AdminUserDetails) => void;
}) {
	const canEditLocal = localMode && user.auth_provider === "local";

	return (
		<tr>
			<td style={tdStyle}>
				<UserIdentity user={user} />
			</td>
			<td style={tdStyle}>
				<Badge>{user.auth_provider === "local" ? "Local" : "SSO"}</Badge>
			</td>
			<td style={tdStyle}>
				<PermissionBadges roots={commonRoots} permissions={user.folder_permissions} />
			</td>
			<td style={tdStyle}>
				<div style={{ display: "flex", gap: "var(--space-1)", flexWrap: "wrap" }}>
					<Badge>{user.passkey_count} passkey{user.passkey_count === 1 ? "" : "s"}</Badge>
					<Badge>{user.totp_enabled ? "TOTP" : "No TOTP"}</Badge>
					<Badge>{user.trusted_device_count} trusted</Badge>
				</div>
			</td>
			<td style={tdStyle}>
				<div style={{ display: "grid", gap: 2 }}>
					<span>{new Date(user.created_at).toLocaleDateString()}</span>
					<span style={{ color: "var(--color-fg-muted)", fontSize: "var(--text-xs)" }}>
						Last login {dateOrNever(user.last_login_at)}
					</span>
				</div>
			</td>
			<td style={tdStyle}>
				{canEditLocal && (
					<div
						style={{
							display: "flex",
							justifyContent: "flex-end",
							gap: "var(--space-1)",
							flexWrap: "wrap",
						}}
					>
						<button
							type="button"
							onClick={() => onEdit(user)}
							style={actionButtonStyle}
						>
							<Icon name="settings" size={16} />
							Edit
						</button>
						<button
							type="button"
							disabled={resetPending}
							onClick={() => onReset(user)}
							style={actionButtonStyle}
						>
							<Icon name="fileLock" size={16} />
							Reset
						</button>
						<button
							type="button"
							disabled={securityLoading}
							onClick={() => onOpenSecurity(user)}
							style={actionButtonStyle}
						>
							<Icon name="settings" size={16} />
							Security
						</button>
					</div>
				)}
			</td>
		</tr>
	);
}

function UserEditModal({
	user,
	commonRoots,
	onClose,
}: {
	user: AdminUserDetails;
	commonRoots: Root[];
	onClose: () => void;
}) {
	const queryClient = useQueryClient();
	const [displayName, setDisplayName] = useState(user.display_name);
	const [isAdmin, setIsAdmin] = useState(user.is_admin);
	const [hasHome, setHasHome] = useState(user.has_home);
	const [permissions, setPermissions] = useState<Record<string, FolderCaps>>(
		user.folder_permissions,
	);

	useEffect(() => {
		setDisplayName(user.display_name);
		setIsAdmin(user.is_admin);
		setHasHome(user.has_home);
		setPermissions(user.folder_permissions);
	}, [user]);

	const saveMutation = useMutation({
		mutationFn: () =>
			api.updateLocalUser(user.id, {
				display_name: displayName.trim() || undefined,
				is_admin: isAdmin,
				has_home: hasHome,
				folder_permissions: normalizePermissions(commonRoots, permissions),
			}),
		onSuccess: () => {
			queryClient.invalidateQueries({ queryKey: ["admin-users"] });
			onClose();
		},
	});

	return (
		<Modal title={`Edit ${user.display_name}`} onClose={onClose}>
			<div style={{ display: "grid", gap: "var(--space-4)" }}>
				<label style={{ display: "grid", gap: "var(--space-1)" }}>
					<span style={fieldLabelStyle}>Display name</span>
					<input
						value={displayName}
						onChange={(e) => setDisplayName(e.target.value)}
						style={inputStyle}
					/>
				</label>
				<div style={{ display: "flex", gap: "var(--space-3)", flexWrap: "wrap" }}>
					<label style={checkboxLabelStyle}>
						<input
							type="checkbox"
							checked={isAdmin}
							onChange={(e) => setIsAdmin(e.target.checked)}
						/>
						Admin
					</label>
					<label style={checkboxLabelStyle}>
						<input
							type="checkbox"
							checked={hasHome}
							onChange={(e) => setHasHome(e.target.checked)}
						/>
						Home folder
					</label>
				</div>
				<PermissionMatrix
					roots={commonRoots}
					permissions={permissions}
					onChange={(rootKey, field, value) =>
						setPermissions((current) =>
							withPermission(current, rootKey, field, value),
						)
					}
				/>
				{saveMutation.error && (
					<span style={errorTextStyle}>{String(saveMutation.error)}</span>
				)}
				<div style={modalActionsStyle}>
					<button type="button" onClick={onClose} style={actionButtonStyle}>
						Cancel
					</button>
					<button
						type="button"
						disabled={saveMutation.isPending}
						onClick={() => saveMutation.mutate()}
						style={{
							...actionButtonStyle,
							background: "var(--color-accent)",
							borderColor: "var(--color-accent)",
							color: "var(--color-accent-fg)",
						}}
					>
						<Icon name="checkCircle" size={16} />
						Save changes
					</button>
				</div>
			</div>
		</Modal>
	);
}

function UserIdentity({ user }: { user: AdminUserDetails }) {
	return (
		<div
			style={{
				display: "flex",
				alignItems: "center",
				gap: "var(--space-2)",
			}}
		>
			{user.picture_url ? (
				<img
					src={user.picture_url}
					alt=""
					style={{
						width: 24,
						height: 24,
						borderRadius: "var(--radius-full)",
						objectFit: "cover",
					}}
				/>
			) : (
				<div
					style={{
						width: 24,
						height: 24,
						borderRadius: "var(--radius-full)",
						background: "var(--color-accent-muted)",
						display: "flex",
						alignItems: "center",
						justifyContent: "center",
						fontSize: "var(--text-xs)",
						fontWeight: 600,
						color: "var(--color-accent)",
					}}
				>
					{user.display_name[0]?.toUpperCase()}
				</div>
			)}
			<div style={{ display: "grid", gap: 2 }}>
				<span>{user.display_name}</span>
				<code style={{ fontSize: "var(--text-xs)", color: "var(--color-fg-muted)" }}>
					{user.username}
				</code>
			</div>
		</div>
	);
}

function PermissionMatrix({
	roots,
	permissions,
	onChange,
	compact = false,
}: {
	roots: Root[];
	permissions: Record<string, FolderCaps>;
	onChange: (rootKey: string, field: keyof FolderCaps, value: boolean) => void;
	compact?: boolean;
}) {
	if (roots.length === 0) return null;

	return (
		<div
			style={{
				display: "grid",
				gap: compact ? "var(--space-1)" : "var(--space-2)",
			}}
		>
			{roots.map((root) => {
				const caps = permissions[root.key] ?? emptyCaps;
				return (
					<div
						key={root.key}
						style={{
							display: "grid",
							gridTemplateColumns: compact
								? "minmax(120px, 1fr) repeat(3, auto)"
								: "minmax(160px, 1fr) repeat(3, auto)",
							alignItems: "center",
							gap: compact ? "var(--space-2)" : "var(--space-3)",
							fontSize: "var(--text-xs)",
						}}
					>
						<span style={{ color: "var(--color-fg-muted)" }}>
							{root.display_name}
						</span>
						<PermissionCheckbox
							label="Read"
							checked={caps.read}
							onChange={(value) => onChange(root.key, "read", value)}
						/>
						<PermissionCheckbox
							label="Write"
							checked={caps.write}
							onChange={(value) => onChange(root.key, "write", value)}
						/>
						<PermissionCheckbox
							label="Share"
							checked={caps.share}
							onChange={(value) => onChange(root.key, "share", value)}
						/>
					</div>
				);
			})}
		</div>
	);
}

function PermissionCheckbox({
	label,
	checked,
	onChange,
}: {
	label: string;
	checked: boolean;
	onChange: (value: boolean) => void;
}) {
	return (
		<label style={checkboxLabelStyle}>
			<input
				type="checkbox"
				checked={checked}
				onChange={(e) => onChange(e.target.checked)}
			/>
			{label}
		</label>
	);
}

function PasswordResult({
	label,
	password,
}: {
	label: string;
	password: string;
}) {
	return (
		<div style={panelStyle}>
			<div
				style={{
					display: "flex",
					alignItems: "center",
					gap: "var(--space-2)",
					marginBottom: "var(--space-2)",
					fontWeight: 600,
				}}
			>
				<Icon name="fileLock" size={16} />
				{label}
			</div>
			<code
				style={{
					display: "block",
					padding: "var(--space-2)",
					borderRadius: "var(--radius-md)",
					background: "var(--color-bg-muted)",
					fontSize: "var(--text-sm)",
					overflowWrap: "anywhere",
				}}
			>
				{password}
			</code>
		</div>
	);
}

function SecurityList<T extends PasskeyInfo | TrustedDevice>({
	title,
	empty,
	items,
	onRevoke,
}: {
	title: string;
	empty: string;
	items: T[];
	onRevoke: (id: string) => void;
}) {
	return (
		<div style={{ marginTop: "var(--space-3)" }}>
			<strong style={{ fontSize: "var(--text-sm)" }}>{title}</strong>
			{items.length === 0 ? (
				<div style={{ ...tdStyle, color: "var(--color-fg-muted)" }}>{empty}</div>
			) : (
				<table style={{ ...tableStyle, marginTop: "var(--space-2)" }}>
					<tbody>
						{items.map((item) => (
							<tr key={item.id}>
								<td style={tdStyle}>{item.label || "Unnamed"}</td>
								<td style={tdStyle}>
									{new Date(item.created_at).toLocaleDateString()}
								</td>
								<td style={tdStyle}>
									{item.last_used_at
										? new Date(item.last_used_at).toLocaleDateString()
										: "Never used"}
								</td>
								<td style={{ ...tdStyle, textAlign: "right" }}>
									{!item.revoked_at && (
										<button
											type="button"
											onClick={() => onRevoke(item.id)}
											style={{ ...actionButtonStyle, color: "var(--color-danger)" }}
										>
											<Icon name="x" size={16} />
											Revoke
										</button>
									)}
								</td>
							</tr>
						))}
					</tbody>
				</table>
			)}
		</div>
	);
}

function Badge({ children }: { children: React.ReactNode }) {
	return <span style={badgeStyle}>{children}</span>;
}

function PermissionBadges({
	roots,
	permissions,
}: {
	roots: Root[];
	permissions: Record<string, FolderCaps>;
}) {
	const badges = roots
		.map((root) => {
			const caps = permissions[root.key] ?? emptyCaps;
			const code = [
				caps.read ? "r" : "",
				caps.write ? "w" : "",
				caps.share ? "s" : "",
			].join("");
			return code ? `${root.display_name}:${code}` : null;
		})
		.filter((value): value is string => Boolean(value));

	if (badges.length === 0) {
		return <span style={{ color: "var(--color-fg-muted)" }}>No shared access</span>;
	}

	return (
		<div style={{ display: "flex", gap: "var(--space-1)", flexWrap: "wrap" }}>
			{badges.map((badge) => (
				<Badge key={badge}>{badge}</Badge>
			))}
		</div>
	);
}

function Modal({
	title,
	children,
	onClose,
}: {
	title: string;
	children: React.ReactNode;
	onClose: () => void;
}) {
	return (
		<div style={modalOverlayStyle} role="presentation" onMouseDown={onClose}>
			<div
				style={modalPanelStyle}
				role="dialog"
				aria-modal="true"
				aria-label={title}
				onMouseDown={(event) => event.stopPropagation()}
			>
				<div style={modalHeaderStyle}>
					<strong>{title}</strong>
					<button type="button" onClick={onClose} style={actionButtonStyle}>
						<Icon name="x" size={16} />
						Close
					</button>
				</div>
				{children}
			</div>
		</div>
	);
}

function normalizePermissions(
	roots: Root[],
	permissions: Record<string, FolderCaps>,
): Record<string, FolderCaps> {
	return Object.fromEntries(
		roots.map((root) => [root.key, permissions[root.key] ?? emptyCaps]),
	);
}

function withPermission(
	permissions: Record<string, FolderCaps>,
	rootKey: string,
	field: keyof FolderCaps,
	value: boolean,
): Record<string, FolderCaps> {
	const current = permissions[rootKey] ?? emptyCaps;
	const next: FolderCaps = { ...current, [field]: value };
	if ((field === "write" || field === "share") && value) {
		next.read = true;
	}
	if (field === "read" && !value) {
		next.write = false;
		next.share = false;
	}
	return { ...permissions, [rootKey]: next };
}

function dateOrNever(value: number | null | undefined): string {
	if (!value) return "Never";
	return new Date(value).toLocaleDateString();
}

const emptyCaps: FolderCaps = { read: false, write: false, share: false };

// ---------------------------------------------------------------------------
// Shared components
// ---------------------------------------------------------------------------

function LoadingRows() {
	return (
		<div
			style={{
				display: "flex",
				flexDirection: "column",
				gap: "var(--space-3)",
			}}
		>
			{[...Array(5)].map((_, i) => (
				<div
					// biome-ignore lint/suspicious/noArrayIndexKey: only available
					key={i}
					className="shimmer"
					style={{ width: "100%", height: 40, borderRadius: 8 }}
				/>
			))}
		</div>
	);
}

function EmptyMessage({ text }: { text: string }) {
	return (
		<div
			style={{
				padding: "var(--space-8)",
				textAlign: "center",
				color: "var(--color-fg-muted)",
				fontSize: "var(--text-sm)",
			}}
		>
			{text}
		</div>
	);
}

// ---------------------------------------------------------------------------
// Table styles
// ---------------------------------------------------------------------------

const tableStyle: React.CSSProperties = {
	width: "100%",
	borderCollapse: "collapse",
	fontSize: "var(--text-sm)",
};

const thStyle: React.CSSProperties = {
	textAlign: "left",
	padding: "var(--space-2) var(--space-3)",
	borderBottom: "1px solid var(--color-border)",
	color: "var(--color-fg-muted)",
	fontWeight: 600,
	fontSize: "var(--text-xs)",
	textTransform: "uppercase",
	letterSpacing: "0.05em",
};

const tdStyle: React.CSSProperties = {
	padding: "var(--space-2) var(--space-3)",
	borderBottom: "1px solid var(--color-border-muted)",
	verticalAlign: "middle",
};

const inputStyle: React.CSSProperties = {
	boxSizing: "border-box",
	padding: "var(--space-2) var(--space-3)",
	border: "1px solid var(--color-border)",
	borderRadius: "var(--radius-md)",
	background: "var(--color-bg)",
	color: "var(--color-fg)",
	fontSize: "var(--text-sm)",
	width: "100%",
};

const actionButtonStyle: React.CSSProperties = {
	display: "inline-flex",
	alignItems: "center",
	gap: "var(--space-1)",
	padding: "var(--space-2) var(--space-3)",
	border: "1px solid var(--color-border)",
	borderRadius: "var(--radius-md)",
	background: "transparent",
	color: "var(--color-fg-muted)",
	cursor: "pointer",
	fontSize: "var(--text-sm)",
	fontWeight: 500,
};

const panelStyle: React.CSSProperties = {
	border: "1px solid var(--color-border)",
	borderRadius: "var(--radius-lg)",
	padding: "var(--space-4)",
	background: "var(--color-bg)",
};

const modalOverlayStyle: React.CSSProperties = {
	position: "fixed",
	inset: 0,
	zIndex: 60,
	display: "flex",
	alignItems: "center",
	justifyContent: "center",
	padding: "var(--space-4)",
	background: "var(--color-overlay)",
};

const modalPanelStyle: React.CSSProperties = {
	width: "min(720px, 100%)",
	maxHeight: "min(760px, calc(100vh - 48px))",
	overflow: "auto",
	border: "1px solid var(--color-border)",
	borderRadius: "var(--radius-lg)",
	padding: "var(--space-4)",
	background: "var(--color-bg)",
	boxShadow: "var(--shadow-lg)",
};

const modalHeaderStyle: React.CSSProperties = {
	display: "flex",
	alignItems: "center",
	justifyContent: "space-between",
	gap: "var(--space-3)",
	marginBottom: "var(--space-4)",
};

const modalActionsStyle: React.CSSProperties = {
	display: "flex",
	justifyContent: "flex-end",
	gap: "var(--space-2)",
	flexWrap: "wrap",
};

const fieldLabelStyle: React.CSSProperties = {
	color: "var(--color-fg-muted)",
	fontSize: "var(--text-xs)",
	fontWeight: 600,
	textTransform: "uppercase",
	letterSpacing: "0.05em",
};

const checkboxLabelStyle: React.CSSProperties = {
	display: "inline-flex",
	alignItems: "center",
	gap: "var(--space-1)",
	color: "var(--color-fg-muted)",
	fontSize: "var(--text-sm)",
};

const badgeStyle: React.CSSProperties = {
	display: "inline-flex",
	alignItems: "center",
	padding: "1px 6px",
	borderRadius: "var(--radius-full)",
	background: "var(--color-bg-muted)",
	color: "var(--color-fg-muted)",
	fontSize: "var(--text-xs)",
	fontWeight: 500,
	whiteSpace: "nowrap",
};

const errorTextStyle: React.CSSProperties = {
	color: "var(--color-danger)",
	fontSize: "var(--text-sm)",
};

const expiryOptions = [
	{ label: "1h", value: 60 * 60 },
	{ label: "12h", value: 12 * 60 * 60 },
	{ label: "1d", value: 24 * 60 * 60 },
	{ label: "7d", value: 7 * 24 * 60 * 60 },
	{ label: "14d", value: 14 * 24 * 60 * 60 },
	{ label: "31d", value: 31 * 24 * 60 * 60 },
	{ label: "90d", value: 90 * 24 * 60 * 60 },
];
