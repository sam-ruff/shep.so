// This entry and its fictional data are excluded from the production build.
import fixture from "../../shared/preview.json";
import findFixture from "../../shared/find-preview.json";
import {
  BrowserSettings,
  Workspace,
  type Repository,
  type Fields,
  type Draft,
  type CalendarEntry,
} from "./model";
import { mount } from "./ui";
class PreviewRepository implements Repository {
  preview = true;
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
mount(new Workspace(new PreviewRepository(), new BrowserSettings()));
