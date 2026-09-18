import { useState, useEffect } from "react";
import { CalendarDays } from "lucide-react";
import { SidebarMenuButton } from "@/src/components/ui/sidebar";
import useLocalStorage from "@/src/components/useLocalStorage";
import Link from "next/link";
import { usePostHogClientCapture } from "@/src/features/posthog-analytics/usePostHogClientCapture";

const SEVEN_DAYS_MS = 7 * 24 * 60 * 60 * 1000;
const FIRST_SEEN_KEY = "book-a-call-first-seen";

export const BookACallButton = () => {
  const [mounted, setMounted] = useState(false);
  const capture = usePostHogClientCapture();
  const [firstSeen, setFirstSeen] = useLocalStorage<number | null>(
    FIRST_SEEN_KEY,
    null,
  );

  useEffect(() => {
    setMounted(true);
    if (firstSeen === null) {
      setFirstSeen(Date.now());
    }
  }, [firstSeen, setFirstSeen]);

  // Render nothing during SSR to avoid hydration mismatch.
  // useLocalStorage reads from localStorage on client which may differ from server initialValue.
  if (!mounted) {
    return null;
  }

  // Hide button after 7 days from first seen
  const isExpired =
    firstSeen !== null && Date.now() > firstSeen + SEVEN_DAYS_MS;

  if (isExpired) {
    return null;
  }

  return (
    <SidebarMenuButton asChild>
      <Link
        href="https://cal.com/team/langfuse/welcome-to-langfuse"
        target="_blank"
        rel="noopener noreferrer"
        onClick={() => {
          capture("sidebar:book_a_call_clicked");
        }}
      >
        <CalendarDays className="h-4 w-4" />
        Book a call
      </Link>
    </SidebarMenuButton>
  );
};
