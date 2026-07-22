import { RouterProvider } from "react-router";
import { router } from "./routes";
import { ThemeProvider } from "./components/ThemeContext";
import { AuthProvider } from "./components/AuthContext";
import { IncomingSessionConsentHost } from "./components/IncomingSessionConsentHost";

export default function App() {
  return (
    <AuthProvider>
      <ThemeProvider>
        <IncomingSessionConsentHost />
        <RouterProvider router={router} />
      </ThemeProvider>
    </AuthProvider>
  );
}
