import type { ButtonHTMLAttributes, ReactNode } from "react";

interface Props extends ButtonHTMLAttributes<HTMLButtonElement> { variant?: "primary" | "secondary" | "ghost"; icon?: ReactNode }
export function Button({ variant = "primary", icon, children, className = "", ...props }: Props) {
  return <button className={`button button--${variant} ${className}`} {...props}>{icon}{children}</button>;
}

