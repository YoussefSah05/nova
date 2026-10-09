import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { CaptionPicker } from "./CaptionPicker";

afterEach(cleanup);
describe("caption translation controls", () => {
  it("selects dual mode and a local target without restarting playback", () => {
    const onChange = vi.fn();
    const onTargetChange = vi.fn();
    const onClose = vi.fn();
    render(
      <CaptionPicker
        mode="dual"
        onChange={onChange}
        onTargetChange={onTargetChange}
        onClose={onClose}
        open
        targetLang="fr"
      />
    );
    expect(screen.getByRole("menuitemradio", { name: "Source + translation" })).toHaveAttribute(
      "aria-checked",
      "true"
    );
    fireEvent.click(screen.getByRole("menuitemradio", { name: "Arabic" }));
    expect(onTargetChange).toHaveBeenCalledWith("ar");
    expect(onClose).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("menuitemradio", { name: "Translation" }));
    expect(onChange).toHaveBeenCalledWith("translation");
  });
  it("offers retry on model failures and disables unsupported live captions", () => {
    const retry = vi.fn();
    const props = { mode: "source" as const, onChange: vi.fn(), onClose: vi.fn(), open: true };
    const view = render(
      <CaptionPicker
        {...props}
        translationNote="Model unavailable"
        translationFailed
        onRetryTranslation={retry}
      />
    );
    fireEvent.click(screen.getByRole("menuitem", { name: "Retry translation" }));
    expect(retry).toHaveBeenCalledOnce();
    view.rerender(<CaptionPicker {...props} isLive />);
    expect(screen.getByRole("menuitemradio", { name: "Translation" })).toBeDisabled();
    expect(screen.getByText("Live AI captions are not available yet.")).toBeInTheDocument();
  });
});
