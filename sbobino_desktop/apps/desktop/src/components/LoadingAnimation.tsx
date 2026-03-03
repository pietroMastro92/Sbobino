
import { Sparkles, type LucideIcon } from "lucide-react";

type LoadingAnimationProps = {
  icon?: LucideIcon;
  title?: string;
  description?: string;
  variant?: "transcribing" | "summarizing" | "chat";
};

export function LoadingAnimation({
  icon: Icon = Sparkles,
  title = "Loading...",
  description = "Please wait...",
  variant = "transcribing",
}: LoadingAnimationProps) {
  return (
    <div className="center-empty" style={{ opacity: 0, animation: "fadeIn 0.5s ease-out forwards", height: "100%", maxHeight: "100%" }}>
      <style>
        {`
          @keyframes fadeIn {
            to { opacity: 1; }
          }
          @keyframes pulseGlow {
            0% { box-shadow: 0 4px 14px rgba(0, 0, 0, 0.04); border-color: var(--line); transform: scale(1); }
            50% { box-shadow: 0 0 16px rgba(109, 148, 197, 0.3); border-color: rgba(109, 148, 197, 0.4); transform: scale(1.02); }
            100% { box-shadow: 0 4px 14px rgba(0, 0, 0, 0.04); border-color: var(--line); transform: scale(1); }
          }
        `}
      </style>
      
      <div className="center-empty-icon" style={{ animation: "pulseGlow 2.5s infinite ease-in-out" }}>
        <Icon size={28} />
      </div>
      <h3>{title}</h3>
      <p>{description}</p>
      
      <div style={{ marginTop: "48px", width: "100%", maxWidth: "720px", display: "flex", justifyContent: "center" }}>
        {variant === "transcribing" && (
          <svg viewBox="0 0 720 140" xmlns="http://www.w3.org/2000/svg" style={{ width: "100%", height: "auto" }}>
            <style>
              {`
                .t-line {
                  fill: none;
                  stroke: var(--muted, #8a7e6e);
                  stroke-width: 6;
                  stroke-linecap: round;
                  stroke-dasharray: 100;
                  stroke-dashoffset: 100;
                  opacity: 0.3;
                }
                .t-line-1 { animation: drawTextLine 3.5s ease-in-out infinite 0s; }
                .t-line-2 { animation: drawTextLine 3.5s ease-in-out infinite 0.6s; }
                .t-line-3 { animation: drawTextLine 3.5s ease-in-out infinite 1.2s; }
                
                @keyframes drawTextLine {
                  0% { stroke-dashoffset: 100; opacity: 0; }
                  10% { opacity: 0.3; }
                  35% { stroke-dashoffset: 0; opacity: 0.3; }
                  70% { stroke-dashoffset: 0; opacity: 0; }
                  100% { stroke-dashoffset: 100; opacity: 0; }
                }
              `}
            </style>
            
            <line x1="10" y1="20" x2="710" y2="20" pathLength="100" className="t-line t-line-1" />
            <line x1="10" y1="56" x2="650" y2="56" pathLength="100" className="t-line t-line-2" />
            <line x1="10" y1="92" x2="460" y2="92" pathLength="100" className="t-line t-line-3" />
          </svg>
        )}

        {variant === "summarizing" && (
          <svg viewBox="0 0 200 240" xmlns="http://www.w3.org/2000/svg" style={{ width: "140px", height: "auto" }}>
            <style>
              {`
                .s-doc {
                  fill: var(--surface, #ffffff);
                  stroke: var(--line, #e2e8f0);
                  stroke-width: 4;
                  rx: 12;
                }
                [data-theme="dark"] .s-doc { fill: #1e293b; stroke: #334155; }
                
                .s-line {
                  stroke: var(--muted, #94a3b8);
                  stroke-width: 6;
                  stroke-linecap: round;
                  transition: all 0.3s ease;
                }
                .s-line-1 { animation: s-shrink-1 3s ease-in-out infinite; }
                .s-line-2 { animation: s-shrink-2 3s ease-in-out infinite; }
                .s-line-3 { animation: s-shrink-3 3s ease-in-out infinite; }
                .s-line-4 { animation: s-shrink-4 3s ease-in-out infinite; }
                .s-line-5 { animation: s-fade 3s ease-in-out infinite; }
                
                .s-scanner {
                  fill: url(#s-grad);
                  opacity: 0.8;
                  animation: s-scan 3s cubic-bezier(0.4, 0, 0.2, 1) infinite;
                }
                
                @keyframes s-scan {
                  0% { transform: translateY(-40px); opacity: 0; }
                  15% { opacity: 1; }
                  85% { opacity: 1; }
                  100% { transform: translateY(220px); opacity: 0; }
                }
                @keyframes s-shrink-1 {
                  0%, 30% { stroke-dasharray: 120 140; stroke-dashoffset: 0; stroke: var(--muted, #94a3b8); }
                  50%, 100% { stroke-dasharray: 60 140; stroke-dashoffset: 0; stroke: var(--detail-active-border, #3d8dca); stroke-width: 8; transform: translateY(10px); }
                }
                @keyframes s-shrink-2 {
                  0%, 40% { stroke-dasharray: 140 140; stroke-dashoffset: 0; stroke: var(--muted, #94a3b8); }
                  60%, 100% { stroke-dasharray: 80 140; stroke-dashoffset: 0; stroke: var(--detail-active-border, #3d8dca); stroke-width: 8; transform: translateY(10px); }
                }
                @keyframes s-shrink-3 {
                  0%, 50% { stroke-dasharray: 100 140; stroke-dashoffset: 0; stroke: var(--muted, #94a3b8); }
                  70%, 100% { stroke-dasharray: 50 140; stroke-dashoffset: 0; stroke: var(--detail-active-border, #3d8dca); stroke-width: 8; transform: translateY(10px); }
                }
                @keyframes s-shrink-4 {
                  0%, 60% { stroke-dasharray: 130 140; opacity: 1; }
                  80%, 100% { stroke-dasharray: 130 140; opacity: 0; }
                }
                @keyframes s-fade {
                  0%, 70% { opacity: 1; }
                  90%, 100% { opacity: 0; }
                }
              `}
            </style>
            
            <defs>
              <linearGradient id="s-grad" x1="0" y1="0" x2="0" y2="1">
                <stop offset="0%" stopColor="var(--detail-active-border, #3d8dca)" stopOpacity="0" />
                <stop offset="50%" stopColor="var(--detail-active-border, #3d8dca)" stopOpacity="0.1" />
                <stop offset="100%" stopColor="var(--detail-active-border, #3d8dca)" stopOpacity="0.8" />
              </linearGradient>
            </defs>

            <rect x="10" y="10" width="180" height="220" className="s-doc" />
            
            <g transform="translate(30, 50)">
              <line x1="0" y1="0" x2="140" y2="0" className="s-line s-line-1" />
              <line x1="0" y1="28" x2="140" y2="28" className="s-line s-line-2" />
              <line x1="0" y1="56" x2="140" y2="56" className="s-line s-line-3" />
              <line x1="0" y1="84" x2="140" y2="84" className="s-line s-line-4" />
              <line x1="0" y1="112" x2="100" y2="112" className="s-line s-line-5" />
            </g>
            
            <rect x="2" y="-20" width="196" height="40" className="s-scanner" />
            <line x1="2" y1="20" x2="198" y2="20" stroke="var(--detail-active-border, #3d8dca)" strokeWidth="3" opacity="0.8" className="s-scanner" />
          </svg>
        )}

        {variant === "chat" && (
          <svg viewBox="0 0 200 140" xmlns="http://www.w3.org/2000/svg" style={{ width: "160px", height: "auto" }}>
            <style>
              {`
                .c-bubble {
                  fill: var(--surface, #ffffff);
                  stroke: var(--line, #e2e8f0);
                  stroke-width: 4;
                  stroke-linejoin: round;
                }
                [data-theme="dark"] .c-bubble { fill: #1e293b; stroke: #334155; }
                
                .c-dot {
                  fill: var(--muted, #94a3b8);
                  transform-origin: center;
                }
                [data-theme="dark"] .c-dot { fill: #64748b; }
                
                .c-dot-1 { animation: chatDotBounce 1.4s infinite ease-in-out both 0s; }
                .c-dot-2 { animation: chatDotBounce 1.4s infinite ease-in-out both 0.16s; }
                .c-dot-3 { animation: chatDotBounce 1.4s infinite ease-in-out both 0.32s; }
                
                @keyframes chatDotBounce {
                  0%, 80%, 100% { transform: translateY(0) scale(1); opacity: 0.5; fill: var(--muted, #94a3b8); }
                  40% { transform: translateY(-10px) scale(1.2); opacity: 1; fill: var(--detail-active-border, #3d8dca); }
                }
                @keyframes floatBubble {
                  0%, 100% { transform: translateY(0); }
                  50% { transform: translateY(-6px); }
                }
              `}
            </style>
            
            <g style={{ animation: "floatBubble 4s ease-in-out infinite" }}>
              <path d="M 20 60 C 20 20, 40 10, 100 10 C 160 10, 180 20, 180 60 C 180 100, 160 110, 100 110 C 80 110, 50 110, 20 130 C 25 110, 20 100, 20 60 Z" className="c-bubble" />
              
              <circle cx="65" cy="62" r="8" className="c-dot c-dot-1" />
              <circle cx="100" cy="62" r="8" className="c-dot c-dot-2" />
              <circle cx="135" cy="62" r="8" className="c-dot c-dot-3" />
            </g>
          </svg>
        )}
      </div>
    </div>
  );
}
