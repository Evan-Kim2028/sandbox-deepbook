import type { Metadata } from 'next';
import { Toaster } from 'sonner';
import './globals.css';
import { Providers } from './providers';

export const metadata: Metadata = {
  title: 'DeepBook Sandbox',
  description: 'Experience DeepBook in a forked mainnet environment',
};

export default function RootLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  return (
    <html lang="en">
      <body>
        <Providers>
          {children}
          <Toaster
            position="bottom-right"
            theme="dark"
            toastOptions={{
              style: {
                background: '#141921',
                border: '1px solid #1E2530',
              },
            }}
          />
        </Providers>
      </body>
    </html>
  );
}
