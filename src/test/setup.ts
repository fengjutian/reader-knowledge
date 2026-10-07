import "@testing-library/jest-dom/vitest";
import { cleanup } from "@testing-library/react";
import { afterEach, vi } from "vitest";

// Tauri 运行时在浏览器测试环境中不存在，api 层会抛错，这里统一桩掉 invoke。
vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(async () => undefined),
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async () => () => undefined),
  emit: vi.fn(async () => undefined),
}));

// 系统文件对话框在 jsdom 里不存在；默认表现成「用户取消」，用例自己再 mock 成选中。
vi.mock("@tauri-apps/plugin-dialog", () => ({
  open: vi.fn(async () => null),
}));

afterEach(() => {
  cleanup();
});
