// Preferences → Profiles and sync: the Google connection card, discovery,
// reviewed publication and enrollment dialogs, and first-setup onboarding.
// Controls are real buttons/checkboxes/selects styled like the rest of
// Preferences; dialogs live outside the render tree and redraw themselves.
import type { Workspace } from "./model";
import type { Account, GatewayRepository } from "./provider";
import {
  describeAccess,
  describeRequest,
  type CalendarChoice,
  type GoogleConnection,
  type ProfileGoogleApi,
  type RequestedServices,
} from "./profile_google";
import type { HistoryPort } from "./profile_history";
import {
  ProfileStore,
  type AccountMapping,
  type ProfileSummary,
} from "./profile_store";
import { ProfileDiscovery } from "./profile_discovery";
import { ProfilePublication } from "./profile_publication";
import { ProfileEnrollment, type EnrollmentRow } from "./profile_enrollment";
import {
  ProfilePreferenceDevice,
  ProfileSettingsStore,
  describeSetting,
} from "./profile_settings";
import type { PortableConnection } from "./profile_types";

function node<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  text = "",
  className = "",
) {
  const n = document.createElement(tag);
  n.textContent = text;
  if (className) n.className = className;
  return n;
}
function button(label: string, action: () => void, primary = false) {
  const b = node("button", label, primary ? "button primary" : "button");
  b.type = "button";
  b.onclick = action;
  return b;
}
function checkbox(
  label: string,
  checked: boolean,
  onChange: (v: boolean) => void,
) {
  const wrap = node("label", "", "checkbox-field");
  const input = node("input");
  input.type = "checkbox";
  input.checked = checked;
  input.setAttribute("aria-label", label);
  input.onchange = () => onChange(input.checked);
  wrap.append(input, node("span", label));
  return wrap;
}
function select(
  label: string,
  value: string,
  choices: [string, string][],
  onChange: (v: string) => void,
) {
  const wrap = node("label", "", "select-field");
  wrap.append(node("span", label));
  const input = node("select");
  input.setAttribute("aria-label", label);
  for (const [key, name] of choices) {
    const o = node("option", name);
    o.value = key;
    o.selected = key === value;
    input.append(o);
  }
  input.onchange = () => onChange(input.value);
  wrap.append(input);
  return wrap;
}
function connectionDetails(a: PortableConnection | Account) {
  const list = node("dl", "", "profile-details");
  for (const [label, value] of [
    ["Email", a.email],
    [
      "Incoming",
      `${a.protocol === "Pop3" ? "POP3" : "IMAP"} ${a.host}:${a.port} (${a.incoming_security === "Tls" ? "SSL/TLS" : "STARTTLS"}, ${a.incoming_auth})`,
    ],
    ["Username", a.username],
    [
      "SMTP",
      `${a.smtp_host}:${a.smtp_port} (${a.smtp_security === "Tls" ? "SSL/TLS" : "STARTTLS"}, ${a.smtp_auth})`,
    ],
    ["SMTP username", a.smtp_username || a.username],
    [
      "Sent copies",
      `${a.sent_copy}${a.sent_folder ? ` in ${a.sent_folder}` : ""}`,
    ],
  ]) {
    list.append(node("dt", label), node("dd", value));
  }
  return list;
}
function dialog(title: string, onClose?: () => void) {
  const d = node("dialog", "", "account-dialog profile-dialog");
  d.setAttribute("aria-label", title);
  const heading = node("div", "", "dialog-heading");
  heading.append(node("h2", title));
  const close = button("Close", () => d.close());
  heading.append(close);
  const body = node("div", "", "account-body");
  const actions = node("div", "", "dialog-actions");
  d.append(heading, body, actions);
  d.addEventListener("close", () => {
    d.remove();
    onClose?.();
  });
  document.body.append(d);
  d.showModal();
  return { dialog: d, body, actions };
}

export interface ProfilesUIOptions {
  workspace: Workspace;
  repository: GatewayRepository;
  api: ProfileGoogleApi;
  settings: ProfileSettingsStore;
  port: HistoryPort;
  identity: string;
  openPreferences: () => void;
}
export class ProfilesUI {
  connection: GoogleConnection | null = null;
  connectionError: string | null = null;
  notice: string | null = null;
  cleanupPending = false;
  choice: RequestedServices = { drive: true, calendar: "off" };
  busy = false;
  store: ProfileStore | null = null;
  discovery: ProfileDiscovery | null = null;
  publication: ProfilePublication | null = null;
  enrollment: ProfileEnrollment | null = null;
  onboarding: { decision: string } | null = null;
  private device: ProfilePreferenceDevice;
  private w: Workspace;
  private repo: GatewayRepository;
  private api: ProfileGoogleApi;
  private port: HistoryPort;
  private identity: string;
  private openPreferences: () => void;
  constructor(options: ProfilesUIOptions) {
    this.w = options.workspace;
    this.repo = options.repository;
    this.api = options.api;
    this.port = options.port;
    this.identity = options.identity;
    this.openPreferences = options.openPreferences;
    this.device = new ProfilePreferenceDevice(
      options.settings,
      options.workspace,
    );
  }
  private changed() {
    this.w.changed();
  }
  async start() {
    const outcome = /^#profiles=([a-z]+)$/.exec(location.hash)?.[1];
    if (outcome) {
      history.replaceState(null, "", location.pathname);
      this.notice =
        {
          connected: "Google connected. Saved permissions are shown below.",
          denied:
            "Google did not grant the requested access. The saved connection is unchanged.",
          failed:
            "Google sign-in did not finish. The saved connection is unchanged. Try again.",
          mismatch:
            "That Google account differs from the beta sign-in. The saved connection is unchanged.",
        }[outcome] ?? null;
      this.openPreferences();
    }
    await this.reload();
  }
  async reload() {
    this.connectionError = null;
    try {
      this.connection = await this.api.connection();
      this.choice = this.connection.pending ?? this.connection.requested;
      if (!this.choice.drive && this.choice.calendar === "off")
        this.choice = { drive: true, calendar: "off" };
      if (this.connection.connected && this.connection.granted.drive)
        await this.bind();
      else this.unbind();
    } catch (error) {
      this.connection = null;
      this.connectionError =
        error instanceof Error
          ? error.message
          : "Could not read the Google connection.";
    }
    this.changed();
  }
  private async bind() {
    const connection = this.connection!;
    if (!this.store) {
      this.store = await ProfileStore.open(this.identity);
      this.cleanupPending = await this.store.cleanupPending();
      const scope = {
        namespace: connection.namespace ?? "",
        principal: connection.principal ?? "",
      };
      this.discovery = new ProfileDiscovery(
        this.store,
        this.api,
        this.port,
        scope,
      );
      this.publication = new ProfilePublication(
        this.store,
        this.api,
        this.port,
        this.discovery,
      );
      this.enrollment = new ProfileEnrollment(
        this.store,
        this.port,
        this.discovery,
        {
          accounts: () => this.repo.accounts,
          importAccount: (a) => this.repo.importAccount(a),
          capture: () => this.device.capture(),
          apply: (r) => this.device.apply(r),
          acknowledge: (id) => this.device.acknowledge(id),
        },
      );
      for (const c of [this.discovery, this.publication, this.enrollment])
        c.addEventListener("change", () => this.changed());
      const mappings = await this.store.range<AccountMapping>(
        "mappings",
        "",
        "￿",
        100_000,
      );
      for (const row of mappings)
        if (
          row.value.reconnect &&
          this.repo.accounts.some((a) => a.id === row.value.local)
        )
          this.repo.reconnectRequired.add(row.value.local);
      this.repo.onCredentialActivated = async (id) => {
        const mapping = await this.store?.get<AccountMapping>("mappings", id);
        if (mapping)
          await this.store?.commit([
            {
              store: "mappings",
              key: id,
              value: { ...mapping, reconnect: false },
            },
          ]);
      };
      this.onboarding =
        (await this.store.get<{ decision: string }>("meta", "onboarding")) ??
        null;
      await this.discovery.load();
      await this.publication.load();
      await this.enrollment.load();
      if (this.discovery.state.phase === "idle" && !this.cleanupPending)
        void this.discovery.find();
    }
  }
  private unbind() {
    this.discovery?.dispose();
    this.publication?.dispose();
    this.enrollment?.dispose();
    this.store?.close();
    this.store = null;
    this.discovery = this.publication = this.enrollment = null;
  }
  async connect() {
    if (this.busy) return;
    this.busy = true;
    this.connectionError = null;
    this.changed();
    try {
      const url = await this.api.connect(this.choice);
      location.assign(url);
    } catch (error) {
      this.connectionError =
        error instanceof Error
          ? error.message
          : "Could not start Google sign-in.";
      this.busy = false;
      this.changed();
    }
  }
  async disconnect() {
    if (this.busy) return;
    this.busy = true;
    this.changed();
    try {
      const store = this.store ?? (await ProfileStore.open(this.identity));
      await store.markCleanup();
      await this.api.disconnect();
      this.discovery?.invalidate();
      this.publication?.invalidate();
      this.enrollment?.invalidate();
      await this.cleanupLocal(store);
      this.notice =
        "Google disconnected from this browser. Other devices keep their own grants.";
    } catch (error) {
      this.connectionError =
        error instanceof Error ? error.message : "Could not disconnect Google.";
    } finally {
      this.busy = false;
      this.unbind();
      await this.reload();
    }
  }
  private async cleanupLocal(store: ProfileStore) {
    try {
      await store.cleanup();
      this.cleanupPending = false;
      this.repo.reconnectRequired.clear();
    } catch (error) {
      this.cleanupPending = true;
      this.connectionError =
        error instanceof Error
          ? error.message
          : "Local profile cleanup did not finish.";
    }
  }
  async retryCleanup() {
    const store = this.store ?? (await ProfileStore.open(this.identity));
    await this.cleanupLocal(store);
    if (!this.store) store.close();
    this.changed();
  }
  private async decide(decision: string) {
    await this.store?.commit([
      { store: "meta", key: "onboarding", value: { decision } },
    ]);
    this.onboarding = { decision };
    this.changed();
  }
  /// First-setup offers on the Mail tab: only after completed discovery, on a
  /// browser without accounts and without an earlier decision.
  banner(): HTMLElement | null {
    const d = this.discovery;
    if (
      !d ||
      !this.connection?.connected ||
      !d.complete ||
      this.onboarding ||
      this.repo.accounts.length ||
      (this.publication?.review &&
        this.publication.review.phase !== "review") ||
      this.enrollment?.review
    )
      return null;
    const card = node("section", "", "settings-card onboarding-card");
    card.setAttribute("aria-label", "Set up sync");
    const profiles = d.page;
    if (!profiles.length) {
      card.append(
        node("h2", "Sync accounts and settings"),
        node(
          "p",
          "No profiles were found in this Google account. Turn on sync to keep account definitions and settings in private Google app data for your other Shep installations. You can change this later in Preferences.",
        ),
      );
      const actions = node("div", "", "dialog-actions");
      actions.append(
        button("Not now", () => void this.decide("declined")),
        button(
          "Turn on sync",
          () => {
            void this.decide("accepted").then(() => this.openPublication());
          },
          true,
        ),
      );
      card.append(actions);
      return card;
    }
    if (profiles.length === 1) {
      const p = profiles[0];
      card.append(
        node("h2", `Use “${p.name ?? "Unnamed profile"}” on this browser?`),
        node(
          "p",
          `${p.accounts} ${p.accounts === 1 ? "account" : "accounts"} and ${p.settings} ${p.settings === 1 ? "setting" : "settings"} were found. Enrolling imports account definitions without passwords; reconnect each account to activate it.`,
        ),
      );
      const actions = node("div", "", "dialog-actions");
      actions.append(
        button("Not now", () => void this.decide("declined")),
        button(
          "Use this profile",
          () => {
            void this.decide("enrolled").then(() => this.enroll(p, true));
          },
          true,
        ),
      );
      card.append(actions);
      return card;
    }
    card.append(
      node("h2", "Choose a profile"),
      node(
        "p",
        "Several profiles exist in this Google account. Profiles are never merged automatically.",
      ),
    );
    const list = node("div", "", "profile-list");
    list.setAttribute("role", "list");
    for (const p of profiles.slice(0, 50))
      list.append(this.profileRow(p, true));
    card.append(list);
    const actions = node("div", "", "dialog-actions");
    actions.append(
      button("Not now", () => void this.decide("declined")),
      button("Create a separate profile", () => {
        void this.decide("accepted").then(() => this.openPublication());
      }),
    );
    card.append(actions);
    return card;
  }
  section(): HTMLElement[] {
    return [this.googleCard(), this.profilesCard()];
  }
  private googleCard(): HTMLElement {
    const card = node("section", "", "settings-card");
    card.setAttribute("aria-label", "Google connection");
    card.append(node("h2", "Profiles and sync"));
    const c = this.connection;
    if (this.notice) {
      const status = node("p", this.notice, "");
      status.role = "status";
      card.append(status);
    }
    if (this.connectionError) {
      const error = node("p", this.connectionError, "");
      error.role = "alert";
      card.append(error);
    }
    if (!c) {
      card.append(
        node("p", "The Google connection could not be read.", "muted"),
      );
      card.append(button("Retry", () => void this.reload()));
      return card;
    }
    const summary = node("div", "", "account-connection google-connection");
    summary.append(
      node("strong", c.email),
      node(
        "span",
        c.connected
          ? `Saved permissions: ${describeAccess(c)}${c.principal ? ` · Drive identity verified` : ""}`
          : "Not connected. Google beta sign-in identifies you; it does not grant Drive or Calendar access.",
      ),
    );
    card.append(summary);
    card.append(
      node(
        "p",
        c.live
          ? "Live Google provider access is configured on this beta server."
          : "Live Google provider access is not connected on this beta server; the connection below uses an isolated fixture provider.",
        "muted",
      ),
    );
    if (!c.available)
      card.append(
        node("p", c.reason ?? "Profile sync is not configured.", "muted"),
      );
    if (c.pending)
      card.append(
        node(
          "p",
          `Waiting for Google consent for ${describeRequest(c.pending)}.`,
          "muted",
        ),
      );
    const requested = node("div", "", "shortcut-list");
    requested.append(
      node(
        "p",
        `Requested at next sign-in: ${describeRequest(this.choice)}. A changed choice does not change saved access until sign-in succeeds.`,
        "muted",
      ),
    );
    requested.append(
      checkbox("Drive app data for profile sync", this.choice.drive, (v) => {
        this.choice = { ...this.choice, drive: v };
        this.changed();
      }),
      select(
        "Calendar access",
        this.choice.calendar,
        [
          ["off", "Off"],
          ["read", "Read"],
          ["edit", "Edit"],
        ],
        (v) => {
          this.choice = { ...this.choice, calendar: v as CalendarChoice };
          this.changed();
        },
      ),
    );
    card.append(requested);
    const actions = node("div", "", "dialog-actions");
    const connect = button(
      c.connected ? "Reconnect Google" : "Connect Google",
      () => void this.connect(),
      true,
    );
    connect.disabled =
      this.busy ||
      !c.available ||
      (!this.choice.drive && this.choice.calendar === "off");
    actions.append(connect);
    if (c.connected) {
      const disconnect = button(
        "Disconnect Google",
        () => void this.disconnect(),
      );
      disconnect.disabled = this.busy;
      actions.append(disconnect);
    }
    if (this.cleanupPending)
      actions.append(
        button("Retry local cleanup", () => void this.retryCleanup()),
      );
    card.append(actions);
    return card;
  }
  private profilesCard(): HTMLElement {
    const card = node("section", "", "settings-card");
    card.setAttribute("aria-label", "Synced profiles");
    card.append(node("h2", "Synced profiles"));
    const d = this.discovery;
    if (!d) {
      card.append(
        node(
          "p",
          "Connect Google with Drive app data to find profiles saved by your other Shep installations.",
          "muted",
        ),
      );
      return card;
    }
    const s = d.state;
    const status = node(
      "p",
      s.phase === "idle"
        ? "Profiles have not been searched yet."
        : s.failed
          ? `Discovery stopped: ${s.error}`
          : d.complete
            ? `Discovery complete: ${s.scanned} ${s.scanned === 1 ? "file" : "files"} verified.`
            : d.paused
              ? `Discovery paused during ${s.phase}.`
              : `Discovery in progress (${s.phase}, ${s.scanned} verified).`,
    );
    status.role = "status";
    status.setAttribute("aria-label", "Discovery status");
    card.append(status);
    if (!d.complete && s.phase !== "idle")
      card.append(
        node(
          "p",
          "Results are incomplete until discovery finishes. An incomplete or failed listing is never treated as an empty Google account.",
          "muted",
        ),
      );
    const actions = node("div", "", "dialog-actions");
    const find = button("Find profiles", () => void d.find());
    find.disabled = d.busy;
    actions.append(find);
    if (s.failed || (d.paused && !d.complete)) {
      const retry = button(
        d.paused && !s.failed ? "Resume discovery" : "Retry discovery",
        () => void d.resume(),
      );
      retry.disabled = d.busy;
      actions.append(retry);
    }
    if (d.busy && !d.paused)
      actions.append(button("Pause discovery", () => d.pause()));
    card.append(actions);
    if (d.page.length) {
      const list = node("div", "", "profile-list");
      list.setAttribute("role", "list");
      for (const p of d.page) list.append(this.profileRow(p, false));
      card.append(list);
    } else
      card.append(
        node(
          "p",
          d.complete
            ? "No profiles found in this Google account."
            : "No verified profiles yet.",
          "muted",
        ),
      );
    const paging = node("div", "", "dialog-actions");
    paging.append(
      button("First page", () => void d.firstPage()),
      button("Next page", () => void d.nextPage()),
    );
    card.append(paging);
    card.append(this.publicationSummary(), this.enrollmentSummary());
    const create = button("Create profile", () => this.openPublication());
    create.disabled =
      !d.complete ||
      !!(
        this.publication?.review && this.publication.review.phase !== "review"
      );
    card.append(create);
    return card;
  }
  private profileRow(p: ProfileSummary, onboarding: boolean): HTMLElement {
    const row = node("div", "", "account-connection");
    row.setAttribute("role", "listitem");
    const state = p.removed
      ? "Removed"
      : p.nameConflict
        ? "Name conflict"
        : !p.initialized
          ? "Setup incomplete"
          : p.waiting || p.ready
            ? "History incomplete"
            : p.conflicts
              ? `${p.conflicts} ${p.conflicts === 1 ? "conflict" : "conflicts"}`
              : "Ready";
    row.append(
      node("strong", p.name ?? "Unnamed profile"),
      node(
        "span",
        `${p.accounts} ${p.accounts === 1 ? "account" : "accounts"} · ${p.settings} ${p.settings === 1 ? "setting" : "settings"} · ${p.files} ${p.files === 1 ? "file" : "files"} · ${state}`,
      ),
    );
    const use = button(`Use ${p.name ?? "unnamed profile"}`, () => {
      if (onboarding)
        void this.decide("enrolled").then(() => this.enroll(p, true));
      else void this.enroll(p, false);
    });
    use.disabled =
      !p.initialized ||
      p.removed ||
      !!p.waiting ||
      !!p.ready ||
      !this.discovery?.complete ||
      !!this.enrollment?.review;
    row.append(use);
    return row;
  }
  private publicationSummary(): HTMLElement {
    const wrap = node("div");
    const r = this.publication?.review;
    if (!r) return wrap;
    const text =
      r.phase === "review"
        ? `Profile “${r.name}” is being reviewed.`
        : r.phase === "complete"
          ? `Profile “${r.name}” published: ${r.uploaded} ${r.uploaded === 1 ? "file" : "files"} uploaded.`
          : r.failed
            ? `Publication of “${r.name}” stopped: ${r.error}`
            : `${this.publication!.paused ? "Paused" : "Publishing"} “${r.name}”: ${r.staged}/${r.total} staged, ${r.uploaded} uploaded.`;
    const status = node("p", text);
    status.role = "status";
    status.setAttribute("aria-label", "Publication status");
    wrap.append(status);
    const actions = node("div", "", "dialog-actions");
    if (r.phase !== "complete")
      actions.append(button("Open publication", () => this.openPublication()));
    if (r.phase === "complete")
      actions.append(
        button("Dismiss publication", () => void this.publication!.cancel()),
      );
    wrap.append(actions);
    return wrap;
  }
  private enrollmentSummary(): HTMLElement {
    const wrap = node("div");
    const r = this.enrollment?.review;
    if (!r) return wrap;
    const text =
      r.phase === "review"
        ? `Profile “${r.name ?? "Unnamed"}” is ready to review.`
        : r.phase === "complete"
          ? `Profile “${r.name ?? "Unnamed"}” applied: ${r.applied} ${r.applied === 1 ? "item" : "items"} applied, ${r.kept} kept local.`
          : r.failed
            ? `Enrollment stopped: ${r.error}`
            : `${this.enrollment!.paused ? "Paused" : r.phase === "copying" ? "Copying" : "Applying"} “${r.name ?? "Unnamed"}”: ${r.copied} copied, ${r.applied} applied.`;
    const status = node("p", text);
    status.role = "status";
    status.setAttribute("aria-label", "Enrollment status");
    wrap.append(status);
    const actions = node("div", "", "dialog-actions");
    if (r.phase !== "complete")
      actions.append(button("Open enrollment", () => this.openEnrollment()));
    else
      actions.append(
        button("Dismiss enrollment", () => void this.enrollment!.cancel()),
      );
    wrap.append(actions);
    return wrap;
  }
  private async enroll(p: ProfileSummary, automatic: boolean) {
    const e = this.enrollment;
    if (!e) return;
    try {
      await e.prepare(p);
      if (automatic && e.review?.phase === "review")
        await e.approve(true, true);
      else this.openEnrollment();
    } catch (error) {
      this.connectionError =
        error instanceof Error
          ? error.message
          : "Could not prepare this profile.";
      this.changed();
    }
  }
  openPublication() {
    const pub = this.publication;
    if (!pub) return;
    let name = pub.review?.name ?? "";
    let after = 0;
    const details = new Set<number>();
    const {
      dialog: d,
      body,
      actions,
    } = dialog("Create profile", () => pub.removeEventListener("change", draw));
    const draw = () => {
      body.replaceChildren();
      actions.replaceChildren();
      const r = pub.review;
      if (!r) {
        body.append(
          node(
            "p",
            "Choose a name, then review the account definitions and settings that will be saved to private Google app data. Passwords, mail and drafts are never included.",
          ),
        );
        const field = node("label", "", "field");
        field.append(node("span", "Profile name"));
        const input = node("input");
        input.setAttribute("aria-label", "Profile name");
        input.value = name;
        input.oninput = () => (name = input.value);
        field.append(input);
        body.append(field);
        const status = node("p", "", "muted");
        status.role = "status";
        body.append(status);
        actions.append(
          button("Cancel", () => d.close()),
          button(
            "Review",
            () => {
              pub
                .prepare(name, this.repo.accounts, this.device.capture())
                .catch(
                  (error) =>
                    (status.textContent =
                      error instanceof Error
                        ? error.message
                        : "Could not prepare the review."),
                );
            },
            true,
          ),
        );
        return;
      }
      if (r.phase === "review") {
        body.append(
          node(
            "p",
            `Profile “${r.name}”. Review the frozen values; changing accounts or preferences afterwards requires a new review.`,
          ),
        );
        const list = node("div", "", "profile-list");
        list.setAttribute("aria-label", "Accounts to publish");
        for (const row of pub.rows(after)) {
          const item = node("div", "", "account-connection");
          item.append(
            checkbox(
              `Publish ${row.account.email}`,
              row.selected,
              (v) => void pub.chooseAccount(row.position, v),
            ),
            button(
              details.has(row.position)
                ? `Hide details for ${row.account.email}`
                : `Details for ${row.account.email}`,
              () => {
                if (details.has(row.position)) details.delete(row.position);
                else details.add(row.position);
                draw();
              },
            ),
          );
          if (details.has(row.position))
            item.append(connectionDetails(row.account));
          list.append(item);
        }
        if (!r.accounts.length)
          list.append(
            node(
              "p",
              "No mail accounts on this browser yet; only settings will be published.",
              "muted",
            ),
          );
        body.append(list);
        if (r.accounts.length > 50) {
          const paging = node("div", "", "dialog-actions");
          paging.append(
            button("First page", () => {
              after = 0;
              draw();
            }),
            button("Next page", () => {
              if (after + 50 < r.accounts.length) after += 50;
              draw();
            }),
          );
          body.append(paging);
        }
        const settings = node("div", "", "shortcut-list");
        settings.setAttribute("aria-label", "Settings to publish");
        for (const s of r.settings)
          settings.append(
            checkbox(
              describeSetting(s.key, s.value),
              s.selected,
              (v) => void pub.chooseSetting(s.key, v),
            ),
          );
        body.append(node("h3", "Settings"), settings);
        const status = node("p", r.error ?? "", "muted");
        status.role = "alert";
        body.append(status);
        actions.append(
          button(
            "Cancel review",
            () => void pub.cancel().then(() => d.close()),
          ),
          button(
            "Publish",
            () => {
              pub
                .approve(this.repo.accounts, this.device.capture())
                .catch((error) => {
                  status.textContent =
                    error instanceof Error
                      ? error.message
                      : "Could not publish.";
                });
            },
            true,
          ),
        );
        return;
      }
      const progress = node(
        "p",
        r.phase === "complete"
          ? `Published ${r.uploaded} ${r.uploaded === 1 ? "file" : "files"} to Google app data.`
          : `${r.staged}/${r.total} operations staged, ${r.uploaded} uploaded.${pub.paused ? " Paused; browse mail and resume later." : ""}`,
      );
      progress.role = "status";
      progress.setAttribute("aria-label", "Publication progress");
      body.append(progress);
      if (r.failed) {
        const alert = node("p", r.error ?? "Publication failed.");
        alert.role = "alert";
        body.append(alert);
      }
      if (r.phase === "complete")
        actions.append(button("Done", () => d.close()));
      else {
        actions.append(button("Browse mail", () => d.close()));
        if (r.failed)
          actions.append(
            button("Retry publication", () => void pub.retry().catch(() => {})),
          );
        else if (pub.busy && !pub.paused)
          actions.append(button("Pause publication", () => pub.pause()));
        else
          actions.append(
            button(
              "Resume publication",
              () => void pub.resume().catch(() => {}),
            ),
          );
      }
    };
    pub.addEventListener("change", draw);
    draw();
  }
  openEnrollment() {
    const e = this.enrollment;
    if (!e) return;
    let after = 0;
    const details = new Set<number>();
    let accounts = e.review?.includeAccounts ?? true;
    let settings = e.review?.includeSettings ?? true;
    const {
      dialog: d,
      body,
      actions,
    } = dialog("Use profile", () => e.removeEventListener("change", draw));
    const rowLabel = (row: EnrollmentRow) =>
      row.kind === "account"
        ? `Import ${row.account!.email}`
        : row.kind === "name"
          ? `Name “${row.name}”`
          : describeSetting(row.key!, row.reset ? undefined : row.value);
    const draw = () => {
      body.replaceChildren();
      actions.replaceChildren();
      const r = e.review;
      if (!r) {
        body.append(node("p", "No enrollment is open."));
        actions.append(button("Close", () => d.close()));
        return;
      }
      if (r.phase === "review") {
        body.append(
          node(
            "p",
            `Profile “${r.name ?? "Unnamed"}”. Accounts are added without passwords and show Reconnect required until you activate them. Cached mail and drafts on this browser are kept.`,
          ),
        );
        const categories = node("div", "", "shortcut-list");
        categories.append(
          checkbox("Apply account definitions", accounts, (v) => {
            accounts = v;
          }),
          checkbox("Apply settings", settings, (v) => {
            settings = v;
          }),
        );
        body.append(categories);
        const list = node("div", "", "profile-list");
        list.setAttribute("aria-label", "Profile contents");
        for (const row of e.rows(after)) {
          const item = node("div", "", "account-connection");
          const box = checkbox(
            rowLabel(row),
            row.selected,
            (v) => void e.choose(row.position, v),
          );
          (box.firstElementChild as HTMLInputElement).disabled = !row.available;
          item.append(box);
          if (row.reason) item.append(node("span", row.reason, "muted"));
          if (row.kind === "account")
            item.append(
              button(
                details.has(row.position)
                  ? `Hide details for ${row.account!.email}`
                  : `Details for ${row.account!.email}`,
                () => {
                  if (details.has(row.position)) details.delete(row.position);
                  else details.add(row.position);
                  draw();
                },
              ),
            );
          if (details.has(row.position) && row.account)
            item.append(connectionDetails(row.account));
          list.append(item);
        }
        body.append(list);
        if (r.rows.length > 50) {
          const paging = node("div", "", "dialog-actions");
          paging.append(
            button("First page", () => {
              after = 0;
              draw();
            }),
            button("Next page", () => {
              if (after + 50 < r.rows.length) after += 50;
              draw();
            }),
          );
          body.append(paging);
        }
        const status = node("p", "", "muted");
        status.role = "alert";
        body.append(status);
        actions.append(
          button(
            "Cancel enrollment",
            () => void e.cancel().then(() => d.close()),
          ),
          button(
            "Apply",
            () => {
              e.approve(accounts, settings).catch((error) => {
                status.textContent =
                  error instanceof Error ? error.message : "Could not apply.";
              });
            },
            true,
          ),
        );
        return;
      }
      const progress = node(
        "p",
        r.phase === "complete"
          ? `Applied ${r.applied} ${r.applied === 1 ? "item" : "items"}; ${r.kept} kept this browser's newer value. Reconnect imported accounts in Mail accounts to activate them.`
          : r.phase === "copying"
            ? `Copying original records: ${r.copied} copied.${e.paused ? " Paused." : ""}`
            : `Applying: ${r.applied} applied.${e.paused ? " Paused; browse mail and resume later." : ""}`,
      );
      progress.role = "status";
      progress.setAttribute("aria-label", "Enrollment progress");
      body.append(progress);
      if (r.failed) {
        const alert = node("p", r.error ?? "Enrollment failed.");
        alert.role = "alert";
        body.append(alert);
      }
      if (r.phase === "complete")
        actions.append(button("Done", () => d.close()));
      else {
        actions.append(button("Browse mail", () => d.close()));
        if (r.failed)
          actions.append(
            button("Retry enrollment", () => void e.retry().catch(() => {})),
          );
        else if (e.busy && !e.paused)
          actions.append(button("Pause enrollment", () => e.pause()));
        else
          actions.append(
            button("Resume enrollment", () => void e.resume().catch(() => {})),
          );
      }
    };
    e.addEventListener("change", draw);
    draw();
  }
  dispose() {
    this.unbind();
  }
}
