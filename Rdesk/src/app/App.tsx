import { RouterProvider } from "react-router";
import { router } from "./routes";
import { ThemeProvider } from "./components/ThemeContext";
import { AuthProvider } from "./components/AuthContext";

export default function App() {
  return (
    <AuthProvider>
      <ThemeProvider>
        <RouterProvider router={router} fallbackElement={<div />} />
      </ThemeProvider>
    </AuthProvider>
  );
}