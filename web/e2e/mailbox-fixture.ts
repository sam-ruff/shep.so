import { expect, type Page } from "@playwright/test";
export const profile = "C".repeat(43);
export const subject = (i: number) =>
  `Selection letter ${i.toString().padStart(3, "0")}`;
export async function seed(page: Page) {
  await page.route("**/api/session", (r) =>
    r.fulfill({
      json: {
        email: "owner@example.test",
        user_id: profile,
        csrf: "X".repeat(43),
      },
    }),
  );
  await page.route("**/seed-selection", (r) =>
    r.fulfill({
      contentType: "text/html",
      body: "<!doctype html><title>Selection fixture setup</title>",
    }),
  );
  await page.goto("/seed-selection");
  await page.evaluate(async (profile) => {
    const path = "/src/storage.ts",
      { BrowserStore } = await import(path),
      store = await BrowserStore.open(profile);
    const changes: any[] = ["work", "personal"].map((id) => ({
      store: "accounts",
      key: id,
      value: {
        id,
        name: id,
        email: `${id}@example.test`,
        protocol: "Pop3",
        host: "mail.example.test",
        port: 995,
        username: id,
        incoming_security: "Tls",
        incoming_auth: "Password",
        smtp_host: "mail.example.test",
        smtp_port: 465,
        smtp_username: id,
        smtp_security: "Tls",
        smtp_auth: "Automatic",
        smtp_separate_password: false,
        sent_copy: "LocalOnly",
        sent_folder: "Sent",
      },
    }));
    for (let i = 0; i < 125; i++) {
      const id = `m${i.toString().padStart(3, "0")}`,
        text = `Body for selection letter ${i}.`;
      const core = {
        id,
        account_id: i % 2 ? "personal" : "work",
        remote_id: id,
        folder: "INBOX",
        sender: "Sender <sender@example.test>",
        recipient: "work@example.test",
        subject: `Selection letter ${i.toString().padStart(3, "0")}`,
        preview: text,
        timestamp: 1788692400 - i,
        unread: true,
        starred: false,
        attachment_count: 0,
      };
      changes.push(
        { store: "mail", key: id, value: { core, text } },
        {
          store: "raw",
          key: id,
          value: btoa(
            `Subject: ${core.subject}\r\nContent-Type: text/plain\r\n\r\n${text}`,
          ),
        },
      );
    }
    await store.commit(changes);
    store.close();
  }, profile);
  await page.goto("/");
  await expect(
    page.getByRole("button", { name: subject(0), exact: true }),
  ).toBeVisible();
}
