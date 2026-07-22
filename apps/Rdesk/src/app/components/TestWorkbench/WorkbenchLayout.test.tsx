import { render, screen } from "@testing-library/react";
import { MemoryRouter, Route, Routes } from "react-router";
import { describe, expect, it } from "vitest";
import { getMockInvoke } from "../../../test/mocks/tauri";
import { WorkbenchLayout } from "./WorkbenchLayout";

function renderWorkbench() {
  getMockInvoke().mockImplementation((command: string) => {
    if (command === "get_system_resource_snapshot") {
      return Promise.resolve(null);
    }
    return Promise.resolve(null);
  });

  return render(
    <MemoryRouter initialEntries={["/test/matrix"]}>
      <Routes>
        <Route path="/test" element={<WorkbenchLayout />}>
          <Route path="matrix" element={<div>Matrix page content</div>} />
        </Route>
      </Routes>
    </MemoryRouter>
  );
}

describe("WorkbenchLayout scrolling", () => {
  it("keeps the workbench chrome fixed and limits scrolling to inner panes", () => {
    const { container } = renderWorkbench();
    const root = container.firstElementChild;

    expect(root).toHaveClass("overflow-hidden");
    expect(screen.getByRole("navigation")).toHaveClass("overflow-y-auto");
    expect(screen.getByRole("main")).toHaveClass("overflow-auto");
  });
});

describe("WorkbenchLayout theme", () => {
  it("uses a route-scoped softened dark theme instead of hard black workbench chrome", () => {
    const { container } = renderWorkbench();
    const root = container.firstElementChild;

    expect(root).toHaveClass("workbench-theme");
    expect(root).not.toHaveClass("dark:bg-[#070a10]");
    expect(screen.getByRole("banner")).not.toHaveClass("dark:bg-[#0d1118]/95");
    expect(screen.getByRole("complementary")).not.toHaveClass("dark:bg-[#0a0e15]");
    expect(screen.getByRole("main")).not.toHaveClass("dark:bg-[#070a10]");
  });

  it("keeps the active workbench navigation in the app brand color instead of white on black", () => {
    renderWorkbench();

    const activeMatrixLink = screen.getByRole("link", { name: /matrix/i });

    expect(activeMatrixLink).toHaveClass("dark:bg-blue-600");
    expect(activeMatrixLink).toHaveClass("dark:text-white");
    expect(activeMatrixLink).not.toHaveClass("dark:bg-white");
    expect(activeMatrixLink).not.toHaveClass("dark:text-black");
  });
});
