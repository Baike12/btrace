import React from "react";
import { Button } from "@/src/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/src/components/ui/card";
import { X } from "lucide-react";
import useLocalStorage from "../useLocalStorage";
import Link from "next/link";
import { usePostHogClientCapture } from "@/src/features/posthog-analytics/usePostHogClientCapture";

const TWO_WEEKS_MS = 14 * 24 * 60 * 60 * 1000;

type SidebarNotification = {
  id: string; // Add unique ID for each notification
  title: string;
  description: React.ReactNode;
  createdAt?: string; // optional, used to expire the notification
  link?: string;
  // defaults to "Learn more" if no linkContent and no linkTitle
  linkTitle?: string;
  linkContent?: React.ReactNode;
  // Time-to-live in milliseconds from createdAt. Defaults to TWO_WEEKS_MS.
  ttlMs?: number;
};

/**
 * Upstream Langfuse shipped this list full of its own launch-week announcements
 * and a GitHub-star promo. btrace has no upstream announcements to make, so the
 * list is intentionally empty and the stack renders nothing. Add entries here
 * when this deployment has something to tell its users.
 */
export const notifications: SidebarNotification[] = [];

const STORAGE_KEY = "dismissed-sidebar-notifications";

export function SidebarNotifications() {
  const capture = usePostHogClientCapture();

  // Defer client-only state until after hydration to avoid SSR mismatch.
  // useLocalStorage returns [] on server but real data on client,
  // which can change the visibility of the notification stack.
  const [hydrated, setHydrated] = React.useState(false);
  React.useEffect(() => {
    setHydrated(true);
  }, []);

  const [dismissedNotifications, setDismissedNotifications] = useLocalStorage<
    string[]
  >(STORAGE_KEY, []);

  const isExpired = (notif: SidebarNotification) => {
    if (!notif.createdAt) return false;
    const created = new Date(notif.createdAt).getTime();
    const ttl = notif.ttlMs ?? TWO_WEEKS_MS;
    return Date.now() > created + ttl;
  };

  const dismissNotification = (id: string) => {
    setDismissedNotifications([...dismissedNotifications, id]);
  };

  // On server / before hydration: show all non-expired (no localStorage awareness).
  // After hydration: also filter out dismissed notifications.
  const activeNotifications = hydrated
    ? notifications.filter(
        (notif) =>
          !dismissedNotifications.includes(notif.id) && !isExpired(notif),
      )
    : notifications.filter((notif) => !isExpired(notif));

  if (activeNotifications.length === 0) {
    return null;
  }

  const MAX_STACK = 3;
  const visibleNotifications = activeNotifications.slice(0, MAX_STACK);
  const frontNotification = visibleNotifications[0];
  const backCount = visibleNotifications.length - 1;
  const peekOffset = 8;
  const peekScaleStep = 0.05;
  const extraBottomPadding = backCount * peekOffset + 2;

  return (
    <div
      className="group-data-[collapsible=icon]:hidden"
      style={{ paddingBottom: extraBottomPadding }}
    >
      <div className="relative">
        {Array.from({ length: backCount }).map((_, i) => {
          const index = i + 1;
          return (
            <Card
              key={`stack-${index}`}
              aria-hidden
              className="bg-card pointer-events-none absolute inset-0 rounded-md shadow-none"
              style={{
                transform: `translateY(${index * peekOffset}px) scaleX(${
                  1 - index * peekScaleStep
                })`,
                transformOrigin: "top center",
                zIndex: visibleNotifications.length - index,
              }}
            />
          );
        })}
        <Card
          key={frontNotification.id}
          className="bg-card relative max-h-60 overflow-hidden rounded-md shadow-none"
          style={{ zIndex: visibleNotifications.length }}
        >
          <Button
            variant="ghost"
            size="sm"
            className="absolute top-2.5 right-1.5 h-5 w-5 p-0"
            onClick={() => {
              capture("notification:dismiss_notification", {
                notification_id: frontNotification.id,
              });
              dismissNotification(frontNotification.id);
            }}
            title="Dismiss"
          >
            <X className="h-3.5 w-3.5" />
          </Button>
          <CardHeader className="px-3 pt-2.5 pr-6 pb-0">
            <CardTitle className="text-sm">{frontNotification.title}</CardTitle>
            <CardDescription className="mt-1">
              {frontNotification.description}
            </CardDescription>
          </CardHeader>
          <CardContent className="px-3 pt-1.5 pb-2.5">
            {frontNotification.link &&
              (frontNotification.linkContent ? (
                <Link
                  href={frontNotification.link}
                  target="_blank"
                  onClick={() => {
                    capture("notification:click_link", {
                      notification_id: frontNotification.id,
                    });
                  }}
                >
                  {frontNotification.linkContent}
                </Link>
              ) : (
                <Button
                  variant="secondary"
                  size="sm"
                  className="w-full"
                  asChild
                >
                  <Link
                    href={frontNotification.link}
                    target="_blank"
                    onClick={() => {
                      capture("notification:click_link", {
                        notification_id: frontNotification.id,
                      });
                    }}
                  >
                    {frontNotification.linkTitle ?? "Learn more"} &rarr;
                  </Link>
                </Button>
              ))}
          </CardContent>
        </Card>
      </div>
    </div>
  );
}
