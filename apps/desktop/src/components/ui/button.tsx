import * as React from "react";
import { Slot } from "@radix-ui/react-slot";
import { cva, type VariantProps } from "class-variance-authority";

import { cn } from "@/lib/utils";

const buttonVariants = cva(
  // Pill shape; a light "sheen" sweeps across on hover via ::before (clipped by
  // overflow-hidden); springy press. Colours come from token vars so the accent
  // theme picker recolours every button live.
  "group/btn relative isolate inline-flex cursor-pointer items-center justify-center gap-2 overflow-hidden whitespace-nowrap rounded-full text-sm font-medium transition-[color,background-color,border-color,box-shadow,transform] duration-200 ease-[cubic-bezier(0.34,1.56,0.64,1)] active:scale-[0.96] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-background disabled:pointer-events-none disabled:opacity-50 before:pointer-events-none before:absolute before:inset-0 before:-translate-x-full before:bg-gradient-to-r before:from-transparent before:via-white/25 before:to-transparent before:transition-transform before:duration-500 hover:before:translate-x-full [&_svg]:pointer-events-none [&_svg]:size-4 [&_svg]:shrink-0 [&_svg]:transition-transform",
  {
    variants: {
      variant: {
        // Brand: token-driven amber gradient + inset top highlight + a glow that
        // blooms on hover.
        default:
          "bg-[linear-gradient(to_bottom,var(--color-brand-hi),var(--color-brand-lo))] text-primary-foreground shadow-[inset_0_1px_0_rgba(255,255,255,0.3),var(--shadow-elev-1)] hover:brightness-[1.05] hover:shadow-[var(--shadow-brand-glow)]",
        // Glass pill.
        secondary:
          "border border-white/10 bg-white/[0.06] text-secondary-foreground shadow-[inset_0_1px_0_rgba(255,255,255,0.08)] backdrop-blur-sm hover:border-white/20 hover:bg-white/[0.10]",
        outline:
          "border border-border bg-transparent before:hidden hover:border-primary/50 hover:bg-white/[0.04] hover:text-foreground",
        ghost: "before:hidden hover:bg-white/5 hover:text-foreground",
        destructive: "bg-destructive text-destructive-foreground shadow-sm hover:bg-destructive/90",
        link: "before:hidden text-primary underline-offset-4 hover:underline",
      },
      size: {
        default: "h-10 px-5 py-2",
        sm: "h-9 px-4",
        lg: "h-11 px-6",
        icon: "h-10 w-10",
        iconSm: "h-8 w-8",
      },
    },
    defaultVariants: {
      variant: "default",
      size: "default",
    },
  }
);

export interface ButtonProps
  extends React.ButtonHTMLAttributes<HTMLButtonElement>, VariantProps<typeof buttonVariants> {
  asChild?: boolean;
}

const Button = React.forwardRef<HTMLButtonElement, ButtonProps>(
  ({ className, variant, size, asChild = false, ...props }, ref) => {
    const Comp = asChild ? Slot : "button";
    return (
      <Comp className={cn(buttonVariants({ variant, size, className }))} ref={ref} {...props} />
    );
  }
);
Button.displayName = "Button";

export { Button, buttonVariants };
