import { afterEach, describe, expect, test } from "bun:test";
import {
  type MessageKey,
  en,
  formatPercent,
  longLabelCatalog,
  setMessages,
  t,
} from "./messages.ts";

afterEach(() => setMessages("en", en));

describe("message catalog", () => {
  test("fills placeholders and formats numbers with Intl", () => {
    expect(t("hud.gold", { gold: 12500 })).toBe("Gold 12,500");
    expect(t("status.trainingRestarted", { seed: "4294967295" })).toBe(
      "Training arena restarted · seed 4294967295",
    );
    expect(formatPercent(1.25)).toBe("125%");
  });

  test("chooses plural forms with Intl.PluralRules", () => {
    expect(t("status.reproduction", { scenario: "arena", count: 1 })).toBe(
      "Reproduction exported · arena · 1 tick",
    );
    expect(t("status.reproduction", { scenario: "arena", count: 2400 })).toBe(
      "Reproduction exported · arena · 2,400 ticks",
    );
  });

  test("falls back to English for a missing message and to the key for an unknown one", () => {
    setMessages("de", { "menu.resume": "Fortsetzen" });
    expect(t("menu.resume")).toBe("Fortsetzen");
    expect(t("menu.leave")).toBe("Leave");
    expect(t("no.such.key" as MessageKey)).toBe("no.such.key");
  });

  test("the long-label locale lengthens every message and keeps its placeholders", () => {
    const long = longLabelCatalog();
    for (const [key, message] of Object.entries(en)) {
      const english = typeof message === "string" ? message : message.other;
      const longer = long[key as MessageKey];
      const lengthened = typeof longer === "string" ? longer : longer.other;
      expect(lengthened.match(/\{\w+\}/g)).toEqual(english.match(/\{\w+\}/g));
      if (/[A-Za-z]/.test(english.replace(/\{\w+\}/g, "")))
        expect(lengthened.length).toBeGreaterThan(english.length);
    }
    setMessages("en", long);
    expect(t("hud.gold", { gold: 3 })).toBe("Góldgól 3");
  });
});
