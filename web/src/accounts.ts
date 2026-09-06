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
    sent_copy: "ServerManaged",
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
    "Passwords stay in this tab until you sign out or reload. Mail and drafts are saved on this browser. The beta does not save Sent copies; use a mail server that saves them automatically.",
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
export function accountPanel(repo: GatewayRepository, changed: () => void) {
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
