export const ogSize = { width: 1200, height: 630 };

export function OgImage({ title, description }: { title: string; description?: string }) {
  return (
    <div
      style={{
        display: 'flex',
        flexDirection: 'column',
        width: '100%',
        height: '100%',
        padding: '64px 72px',
        backgroundColor: '#0e0e11',
        color: '#f5f5f5',
        borderBottom: '16px solid #fb7185',
      }}
    >
      <div style={{ display: 'flex', fontSize: 30, fontWeight: 600, color: '#fb7185' }}>
        tako.rust-dd.com
      </div>
      <div style={{ display: 'flex', marginTop: 28, fontSize: 76, fontWeight: 800, lineHeight: 1.1 }}>
        {title}
      </div>
      {description ? (
        <div
          style={{
            display: 'flex',
            marginTop: 28,
            fontSize: 36,
            lineHeight: 1.35,
            color: 'rgba(245,245,245,0.78)',
          }}
        >
          {description}
        </div>
      ) : null}
      <div style={{ display: 'flex', marginTop: 'auto', fontSize: 30, fontWeight: 600 }}>
        Tako · multi-transport Rust web framework
      </div>
    </div>
  );
}
