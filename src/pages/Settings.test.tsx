import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { Settings } from "./Settings";
import { api } from "../api/tauri";

vi.mock("@tauri-apps/api/app", () => ({ getVersion: vi.fn(async () => "0.5.0") }));

describe("Settings 关于页面", () => {
  beforeEach(() => {
    vi.spyOn(api, "hasSecret").mockResolvedValue(false);
  });

  it("显示运行时版本和软件信息", async () => {
    const user = userEvent.setup();
    render(<Settings/>);

    await user.click(screen.getByRole("tab", { name: "关于" }));

    expect(await screen.findAllByText("v0.5.0")).toHaveLength(2);
    expect(screen.getByText("阅读与知识管理工具")).toBeInTheDocument();
    expect(screen.getByText("Tauri · React · SQLite")).toBeInTheDocument();
  });
});
