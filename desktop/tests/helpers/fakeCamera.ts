import type { Page } from "@playwright/test";

/**
 * Stub getUserMedia with a canvas-generated stream (animated gradient with a
 * moving circle) so the camera phases work headless and deterministically —
 * Playwright's bundled headless shell has no media capture support.
 */
export function installFakeCamera(
  page: Page,
  options: {
    cameraDelayMs?: number;
    failRequests?: number;
    holdCamera?: boolean;
  } = {},
) {
  return page.addInitScript(
    (cameraOptions) => {
      const canvas = document.createElement("canvas");
      canvas.width = 640;
      canvas.height = 480;
      const context = canvas.getContext("2d");
      let hue = 0;
      setInterval(() => {
        if (!context) {
          return;
        }
        hue = (hue + 7) % 360;
        context.fillStyle = `hsl(${hue} 80% 60%)`;
        context.fillRect(0, 0, canvas.width, canvas.height);
        context.fillStyle = "#ffffff";
        context.beginPath();
        context.arc(
          canvas.width / 2 + Math.sin(hue / 30) * 60,
          canvas.height / 2,
          90,
          0,
          Math.PI * 2,
        );
        context.fill();
      }, 90);
      const stream = canvas.captureStream(15);
      const mediaDevices = navigator.mediaDevices ?? ({} as MediaDevices);
      if (!navigator.mediaDevices) {
        Object.defineProperty(navigator, "mediaDevices", {
          configurable: true,
          value: mediaDevices,
        });
      }
      const devices: MediaDeviceInfo[] = [
        {
          deviceId: "builtin-camera",
          groupId: "mac",
          kind: "videoinput",
          label: "FaceTime HD Camera",
          toJSON() {
            return this;
          },
        } as MediaDeviceInfo,
        {
          deviceId: "iphone-continuity",
          groupId: "iphone",
          kind: "videoinput",
          label: "Kenny's iPhone Camera",
          toJSON() {
            return this;
          },
        } as MediaDeviceInfo,
      ];
      const testWindow = window as Window & {
        __BUZZ_E2E_CAMERA_CONSTRAINTS__?: MediaStreamConstraints[];
        __BUZZ_E2E_CAMERA_REQUEST_COUNT__?: number;
        __BUZZ_E2E_RELEASE_CAMERA__?: () => void;
      };
      testWindow.__BUZZ_E2E_CAMERA_CONSTRAINTS__ = [];
      testWindow.__BUZZ_E2E_CAMERA_REQUEST_COUNT__ = 0;
      let releaseCamera: (() => void) | null = null;
      testWindow.__BUZZ_E2E_RELEASE_CAMERA__ = () => {
        releaseCamera?.();
        releaseCamera = null;
      };
      Object.defineProperty(mediaDevices, "enumerateDevices", {
        configurable: true,
        value: () => Promise.resolve(devices),
      });
      Object.defineProperty(mediaDevices, "addEventListener", {
        configurable: true,
        value: () => {},
      });
      Object.defineProperty(mediaDevices, "removeEventListener", {
        configurable: true,
        value: () => {},
      });
      Object.defineProperty(mediaDevices, "getUserMedia", {
        configurable: true,
        value: async (constraints: MediaStreamConstraints) => {
          testWindow.__BUZZ_E2E_CAMERA_CONSTRAINTS__?.push(constraints);
          testWindow.__BUZZ_E2E_CAMERA_REQUEST_COUNT__ =
            (testWindow.__BUZZ_E2E_CAMERA_REQUEST_COUNT__ ?? 0) + 1;
          if (
            testWindow.__BUZZ_E2E_CAMERA_REQUEST_COUNT__ <=
            cameraOptions.failRequests
          ) {
            throw new DOMException("Camera access denied", "NotAllowedError");
          }
          if (cameraOptions.holdCamera) {
            await new Promise<void>((resolve) => {
              releaseCamera = resolve;
            });
          } else if (cameraOptions.cameraDelayMs > 0) {
            await new Promise((resolve) =>
              setTimeout(resolve, cameraOptions.cameraDelayMs),
            );
          }
          return Promise.resolve(stream);
        },
      });
    },
    {
      cameraDelayMs: options.cameraDelayMs ?? 0,
      failRequests: options.failRequests ?? 0,
      holdCamera: options.holdCamera ?? false,
    },
  );
}
