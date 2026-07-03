import type { Metadata } from "next";
import localFont from "next/font/local";
import "./globals.css";

const bebasNeue = localFont({
  variable: "--font-bebas",
  display: "swap",
  src: [
    {
      path: "./fonts/BebasNeue-Latin.woff2",
      weight: "400",
      style: "normal",
    },
  ],
});

const ubuntu = localFont({
  variable: "--font-ubuntu",
  display: "swap",
  src: [
    {
      path: "./fonts/Ubuntu-Latin-300.woff2",
      weight: "300",
      style: "normal",
    },
    {
      path: "./fonts/Ubuntu-Latin-400.woff2",
      weight: "400",
      style: "normal",
    },
    {
      path: "./fonts/Ubuntu-Latin-500.woff2",
      weight: "500",
      style: "normal",
    },
    {
      path: "./fonts/Ubuntu-Latin-700.woff2",
      weight: "700",
      style: "normal",
    },
  ],
});

export const metadata: Metadata = {
  title: "MIRASET Wallet",
  description: "MIRASET desktop wallet",
};

export default function RootLayout({
  children,
}: Readonly<{
  children: React.ReactNode;
}>) {
  return (
    <html lang="en">
      <body className={`${ubuntu.variable} ${bebasNeue.variable} antialiased`}>
        {children}
      </body>
    </html>
  );
}
