import type { RemovalReview } from "./account_removal";
import { GatewayRepository, type Account, type Endpoint } from "./provider";
function node<K extends keyof HTMLElementTagNameMap>(tag: K, text = "") {
  const n = document.createElement(tag);
  n.textContent = text;
  return n;
}
function control(
  name: string,
  value: string,
  choices?: string[],
  type = "text",
) {
  const label = node("label");
  label.className = "field";
  label.append(node("span", name));
  const input = choices ? node("select") : node("input");
  input.name = name;
  input.setAttribute("aria-label", name);
  if (input instanceof HTMLInputElement) {
    input.type = type;
    input.required = true;
    input.autocomplete = type === "password" ? "off" : "on";
  }
  if (choices)
    for (const choice of choices) {
      const labels: Record<string, string> = {
        Imap: "IMAP",
        Pop3: "POP3",
        Tls: "SSL/TLS",
        StartTls: "STARTTLS",
        Password: "Normal password",
        Plain: "SASL PLAIN",
        Login: "LOGIN",
      };
      const option = node("option", labels[choice] ?? choice);
      option.value = choice;
      input.append(option);
    }
  input.value = value;
  label.append(input);
  return label;
}
function button(label: string, action: () => void) {
  const b = node("button", label);
  b.type = "button";
  b.className = "button";
  b.onclick = action;
  return b;
}
function connect(
  repo: GatewayRepository,
  endpoints: Endpoint[],
  changed: () => void,
  existing?: Account,
) {
  const incoming = endpoints.find((e) => e.service !== "smtp");
  const smtp = endpoints.find((e) => e.service === "smtp");
  const account: Account = existing ?? {
    id: crypto.randomUUID(),
    name: "",
    email: "",
    protocol: incoming?.service === "pop3" ? "Pop3" : "Imap",
    host: incoming?.host ?? "",
    port: incoming?.port ?? 993,
    username: "",
    incoming_security: "Tls",
    incoming_auth: "Password",
    smtp_host: smtp?.host ?? "",
    smtp_port: smtp?.port ?? 465,
    smtp_username: "",
    smtp_security: smtp?.port === 587 ? "StartTls" : "Tls",
    smtp_auth: "Automatic",
    smtp_separate_password: false,
    sent_copy: "Automatic",
    sent_folder: "",
  };
  const dialog = node("dialog");
  dialog.className = "account-dialog";
  dialog.setAttribute(
    "aria-label",
    existing ? `Reconnect ${account.email}` : "Add mail account",
  );
  const form = node("form");
  form.append(
    node("h2", existing ? `Reconnect ${account.email}` : "Add mail account"),
  );
  const description = node(
    "p",
    "Passwords stay in this tab until you sign out or reload. Mail and drafts are saved on this browser. Configure Sent copies separately in Mail accounts.",
  );
  description.className = "muted";
  form.append(description);
  const body = node("div");
  body.className = "account-body";
  form.append(body);
  const settings = node("fieldset");
  settings.className = "account-fields";
  settings.append(
    control("Account name", account.name),
    control("Email address", account.email, undefined, "email"),
    control("Incoming protocol", account.protocol, ["Imap", "Pop3"]),
    control("Incoming server", account.host),
    control("Incoming port", String(account.port), undefined, "number"),
    control("Incoming username", account.username),
    control("Incoming security", account.incoming_security, [
      "Tls",
      "StartTls",
    ]),
    control("Incoming authentication", account.incoming_auth, [
      "Password",
      "Plain",
    ]),
    control("SMTP server", account.smtp_host),
    control("SMTP port", String(account.smtp_port), undefined, "number"),
    control("SMTP username", account.smtp_username),
    control("SMTP security", account.smtp_security, ["Tls", "StartTls"]),
    control("SMTP authentication", account.smtp_auth, [
      "Automatic",
      "Plain",
      "Login",
    ]),
  );
  settings.querySelector<HTMLInputElement>('[name="SMTP username"]')!.required =
    false;
  for (const port of settings.querySelectorAll<HTMLInputElement>(
    'input[type="number"]',
  )) {
    port.min = "1";
    port.max = "65535";
  }
  settings.disabled = !!existing;
  body.append(
    settings,
    control("Incoming password", "", undefined, "password"),
  );
  const separate = node("label");
  separate.className = "checkbox-field";
  const check = node("input");
  check.type = "checkbox";
  check.checked = account.smtp_separate_password;
  separate.append(
    check,
    document.createTextNode("Use a different SMTP password"),
  );
  const smtpSecret = control("SMTP password", "", undefined, "password");
  function toggle() {
    smtpSecret.hidden = !check.checked;
    smtpSecret.querySelector("input")!.required = check.checked;
  }
  check.onchange = toggle;
  toggle();
  body.append(separate, smtpSecret);
  const status = node("p");
  status.role = "status";
  const actions = node("div");
  actions.className = "dialog-actions";
  const close = button("Cancel", () => dialog.close());
  const save = node("button", "Verify and save account");
  save.type = "submit";
  save.className = "button primary";
  actions.append(close, save);
  form.append(status, actions);
  dialog.append(form);
  let busy = false;
  dialog.addEventListener("cancel", (e) => {
    if (busy) e.preventDefault();
  });
  dialog.addEventListener("close", () => {
    for (const p of form.querySelectorAll<HTMLInputElement>(
      'input[type="password"]',
    ))
      p.value = "";
    dialog.remove();
  });
  form.onsubmit = async (e) => {
    e.preventDefault();
    if (busy || !form.reportValidity()) return;
    busy = true;
    save.disabled = close.disabled = true;
    status.textContent = "Checking incoming mail and SMTP…";
    const value = (name: string) =>
      form.querySelector<HTMLInputElement | HTMLSelectElement>(
        `[name="${name}"]`,
      )!.value;
    const next: Account = existing ?? {
      ...account,
      name: value("Account name"),
      email: value("Email address"),
      protocol: value("Incoming protocol") as Account["protocol"],
      host: value("Incoming server"),
      port: Number(value("Incoming port")),
      username: value("Incoming username"),
      incoming_security: value(
        "Incoming security",
      ) as Account["incoming_security"],
      incoming_auth: value(
        "Incoming authentication",
      ) as Account["incoming_auth"],
      smtp_host: value("SMTP server"),
      smtp_port: Number(value("SMTP port")),
      smtp_username: value("SMTP username"),
      smtp_security: value("SMTP security") as Account["smtp_security"],
      smtp_auth: value("SMTP authentication") as Account["smtp_auth"],
      smtp_separate_password: check.checked,
    };
    const editable = [
      ...body.querySelectorAll<HTMLInputElement | HTMLSelectElement>(
        "input,select",
      ),
    ].filter((input) => !input.disabled);
    for (const input of editable) input.disabled = true;
    try {
      await repo.connect(
        next,
        value("Incoming password"),
        check.checked ? value("SMTP password") : value("Incoming password"),
      );
      dialog.close();
      changed();
    } catch (error) {
      status.textContent =
        error instanceof Error
          ? error.message
          : "Could not verify the connection. Check the settings and retry.";
    } finally {
      busy = false;
      for (const input of editable) input.disabled = false;
      save.disabled = close.disabled = false;
    }
  };
  document.body.append(dialog);
  dialog.showModal();
}
function sentPreferences(
  repo: GatewayRepository,
  account: Account,
  changed: () => void,
) {
  const dialog = node("dialog");
  dialog.className = "account-dialog sent-preferences";
  dialog.setAttribute("aria-label", `Sent copies for ${account.email}`);
  const form = node("form");
  const title = node("h2", "Sent copies");
  const policy = control("Sent-copy policy", account.sent_copy, [
    "Automatic",
    "ServerManaged",
    "LocalOnly",
  ]);
  const select = policy.querySelector("select")!;
  const labels = [
    "Save a copy on the mail server",
    "My server saves Sent automatically",
    "Keep Sent copies on this browser",
  ];
  [...select.options].forEach((option, i) => (option.textContent = labels[i]));
  const destination = control("Server Sent folder", account.sent_folder);
  const input = destination.querySelector("input")!;
  input.required = false;
  const hint = node(
    "p",
    account.protocol === "Pop3"
      ? "POP3 keeps Sent copies on this browser."
      : "Leave the folder empty to discover the server's Sent folder. Copying a message does not resend it to recipients.",
  );
  hint.className = "muted";
  const status = node("p");
  status.role = "status";
  const actions = node("div");
  actions.className = "dialog-actions";
  const cancel = button("Cancel", () => dialog.close());
  const save = node("button", "Save Sent preferences");
  save.type = "submit";
  save.className = "button primary";
  actions.append(cancel, save);
  form.append(title, policy, destination, hint, status, actions);
  dialog.append(form);
  dialog.addEventListener("close", () => dialog.remove());
  let busy = false;
  form.onsubmit = async (event) => {
    event.preventDefault();
    if (busy) return;
    busy = true;
    save.disabled = select.disabled = input.disabled = true;
    try {
      await repo.saveSentPreferences(
        account.id,
        select.value as Account["sent_copy"],
        input.value,
      );
      dialog.close();
      changed();
    } catch (error) {
      status.textContent =
        error instanceof Error ? error.message : "Could not save. Retry.";
    } finally {
      busy = false;
      save.disabled = select.disabled = input.disabled = false;
    }
  };
  document.body.append(dialog);
  dialog.showModal();
}
function removeAccount(
  repo: GatewayRepository,
  account: Account,
  changed: (id: string) => void,
) {
  const dialog = node("dialog");
  dialog.className = "account-dialog account-removal";
  dialog.setAttribute("aria-label", `Remove ${account.email}`);
  const title = node("h2", "Remove account");
  const body = node("div");
  body.className = "account-body";
  body.append(
    node("p", account.email),
    node(
      "p",
      "Remove this account and its cached mail, drafts, attached files and delivery records from this browser. Mail on the server is unchanged. Local-only mail and unsent drafts cannot be recovered here after removal.",
    ),
  );
  const counts = node("p");
  const label = node("label");
  label.className = "checkbox-field";
  const check = node("input");
  check.type = "checkbox";
  check.setAttribute(
    "aria-label",
    "Discard unfinished delivery and move records",
  );
  label.append(
    check,
    node(
      "span",
      "Discard unfinished delivery and move records. Removal cannot cancel or undo an operation that reached the server. Check Sent and the source/destination folders first.",
    ),
  );
  const status = node("p");
  status.role = "status";
  const reload = button("Reload removal counts", () => void load());
  const cancel = button("Cancel", () => dialog.close());
  const remove = button("Remove from browser", () => void submit());
  remove.classList.add("danger");
  const actions = node("div");
  actions.className = "dialog-actions";
  actions.append(cancel, remove);
  body.append(counts, label, reload, status);
  dialog.append(title, body, actions);
  let review: RemovalReview | undefined,
    busy = false;
  function state() {
    remove.disabled =
      busy ||
      !review ||
      (!!(review.unresolved || review.moves) && !check.checked);
    reload.disabled = busy;
    cancel.disabled = busy;
    check.disabled = busy;
  }
  check.onchange = state;
  async function load() {
    busy = true;
    review = undefined;
    check.checked = false;
    label.hidden = true;
    status.textContent = "Loading removal counts…";
    state();
    try {
      review = await repo.removalPreview(account.id);
      counts.textContent = `${review.messages} cached messages · ${review.drafts} drafts · ${review.files} draft files · ${review.outgoing} delivery records`;
      label.hidden = !(review.unresolved || review.moves);
      status.textContent = label.hidden
        ? ""
        : `${review.unresolved} unfinished deliveries · ${review.moves} unfinished moves`;
    } catch (e) {
      status.textContent =
        e instanceof Error ? e.message : "Could not load the review. Retry.";
    } finally {
      busy = false;
      state();
    }
  }
  async function submit() {
    if (!review || busy) return;
    busy = true;
    status.textContent = "Removing local account data…";
    state();
    try {
      await repo.removeAccount(review, check.checked);
      dialog.close();
      changed(account.id);
    } catch (e) {
      status.textContent =
        e instanceof Error
          ? e.message
          : "Could not remove this account. Retry.";
    } finally {
      busy = false;
      state();
    }
  }
  dialog.addEventListener("cancel", (e) => {
    if (busy) e.preventDefault();
  });
  dialog.addEventListener("close", () => dialog.remove());
  document.body.append(dialog);
  dialog.showModal();
  void load();
}
export function accountPanel(
  repo: GatewayRepository,
  changed: (removed?: string) => void,
) {
  const panel = node("section");
  panel.className = "settings-card";
  panel.append(node("h2", "Mail accounts"));
  const status = node("p", "Loading available mail servers…");
  status.role = "status";
  panel.append(status);
  const draw = (endpoints: Endpoint[]) => {
    for (const account of repo.accounts) {
      const row = node("div");
      row.className = "account-connection";
      row.append(
        node("strong", account.email),
        node(
          "span",
          repo.connected(account.id)
            ? "Connected in this tab"
            : "Reconnect to refresh or send",
        ),
        button(`Reconnect ${account.email}`, () =>
          connect(repo, endpoints, changed, account),
        ),
      );
      row.append(
        button(`Sent copies for ${account.email}`, () =>
          sentPreferences(
            repo,
            repo.accounts.find((a) => a.id === account.id) ?? account,
            changed,
          ),
        ),
      );
      row.append(
        button(`Remove ${account.email}`, () =>
          removeAccount(repo, account, changed),
        ),
      );
      panel.append(row);
    }
    if (endpoints.length)
      panel.append(
        button("Add mail account", () => connect(repo, endpoints, changed)),
      );
  };
  void repo
    .capabilities()
    .then((c) => {
      status.textContent = c.mail
        ? "Only mail servers enabled by the beta administrator can connect."
        : "Mail connections are not enabled on this beta server yet.";
      draw(c.endpoints);
    })
    .catch((error) => {
      status.textContent =
        error instanceof Error
          ? error.message
          : "Could not load available servers.";
      draw([]);
    });
  return panel;
}
