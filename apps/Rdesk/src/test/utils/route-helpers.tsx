/**
 * Route helpers for page testing
 *
 * Provides utilities to simulate navigation and route state
 * without actually using React Router's navigation.
 */

/**
 * Simulated navigation state for testing
 */
export interface NavigationState {
  currentRoute: string;
  params: Record<string, string>;
}

/**
 * Create a mock navigator for testing
 */
export const createMockNavigator = () => {
  const navigate = vi.fn();

  return {
    navigate,
    getCurrentRoute: () => "/",
    getParam: (key: string) => undefined,
  };
};

/**
 * Simulate being at a specific route with params
 *
 * Usage:
 * const { wrapper } = mockRouteContext("/devices/123", { id: "123" });
 */
export const mockRouteContext = (
  route: string,
  params: Record<string, string> = {}
) => {
  // This can be extended to wrap components in a custom Router context
  // For now, use MemoryRouter in renderPage() instead
  return { route, params };
};
