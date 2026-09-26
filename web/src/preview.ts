// This entry and its fictional data are excluded from the production build.
import { PreviewSelection } from "./preview_selection";
import type { SelectionCommand } from "./selection_types";
import fixture from "../../shared/preview.json";
import findFixture from "../../shared/find-preview.json";
import {
  BrowserSettings,
  Workspace,
  type Repository,
  type Fields,
  type Draft,
  type CalendarEntry,
  type Preferences,
} from "./model";
import { mount } from "./ui";

// The website passes its own appearance so an embedded demo matches the page.
class PreviewSettings extends BrowserSettings {
  read(): Preferences {
    const saved = super.read();
    const appearance = new URLSearchParams(location.search).get("appearance");
    return appearance === "light" ||
      appearance === "dark" ||
      appearance === "system"
      ? { ...saved, appearance }
      : saved;
  }
}

class PreviewRepository implements Repository {
  preview = true;
  homeLink = location.pathname.endsWith("/demo/") ? "../#download" : undefined;
  private selections = new PreviewSelection(() => this.cached);
  selection(command: SelectionCommand, observed: string[] = []) {
    return this.selections.selection(command, observed);
  }
  cached = structuredClone(fixture.messages);
  events = structuredClone(fixture.events);
  constructor() {
    if (new URLSearchParams(location.search).has("find"))
      this.cached[0].body = findFixture.body;
  }
  private async wait() {
    await new Promise((r) => setTimeout(r, 350));
    if (new URLSearchParams(location.search).has("fail"))
      throw new Error("Fixture rejection");
  }
  async refresh() {
    await this.wait();
    return structuredClone(this.cached);
  }
  async mutate(id: string, fields: Fields) {
    await this.wait();
    this.cached = this.cached.map((m) =>
      m.id === id ? { ...m, ...fields } : m,
    );
  }
  async saveDraft(_draft: Draft) {
    await this.wait();
  }
  async send(): Promise<void> {
    throw new Error("Preview cannot send mail. Your draft is still open.");
  }
  async saveEvent(event: CalendarEntry) {
    await this.wait();
    if (this.events.some((e) => e.id === event.id && e.readOnly))
      throw new Error("Read-only calendar");
    this.events = [...this.events.filter((e) => e.id !== event.id), event];
  }
}
// The public demo can share an origin with a real client; keep its settings apart.
mount(
  new Workspace(
    new PreviewRepository(),
    new PreviewSettings("shep.preview.preferences.v1"),
  ),
);
