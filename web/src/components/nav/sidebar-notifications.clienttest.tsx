import { render, screen } from "@testing-library/react";
import { SidebarNotifications, notifications } from "./sidebar-notifications";

const mockState = vi.hoisted(() => ({ dismissed: [] as string[] }));

vi.mock("../useLocalStorage", () => ({
  __esModule: true,
  default: () => [mockState.dismissed, vi.fn()],
}));

describe("SidebarNotifications", () => {
  beforeEach(() => {
    mockState.dismissed = [];
  });

  it("renders nothing when no notifications are configured", () => {
    expect(notifications).toHaveLength(0);

    const { container } = render(<SidebarNotifications />);

    expect(container).toBeEmptyDOMElement();
    expect(screen.queryByTitle("Dismiss")).not.toBeInTheDocument();
  });
});
