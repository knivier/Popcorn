#!/usr/bin/env python3
"""
Popcorn Kernel Builder - Tkinter GUI

Run from anywhere:
  python3 src/build/gui-tk.py
"""

import tkinter as tk
from tkinter import ttk, scrolledtext, messagebox
import subprocess
import threading
import os
import sys
import time
from pathlib import Path

class ModernPopcornBuilder:
    def __init__(self, root):
        self.root = root
        self.root.title("🍿 Popcorn Kernel Builder - x64 Edition")
        self.root.geometry("1000x750")
        self.root.resizable(True, True)
        
        # Modern dark theme colors
        self.colors = {
            'bg': '#1a1a1a',           # Main background
            'surface': '#2d2d2d',      # Card/surface background
            'surface_hover': '#3a3a3a', # Hover state
            'primary': '#00d4aa',      # Primary accent (teal)
            'primary_hover': '#00b894', # Primary hover
            'secondary': '#6c5ce7',    # Secondary accent (purple)
            'success': '#00b894',      # Success green
            'warning': '#fdcb6e',      # Warning orange
            'error': '#e17055',        # Error red
            'text': '#ffffff',         # Primary text
            'text_secondary': '#b2bec3', # Secondary text
            'border': '#404040',       # Border color
            'accent': '#fd79a8'        # Pink accent
        }
        
        # Configure root
        self.root.configure(bg=self.colors['bg'])
        
        # Check if we're in the src directory
        src_dir = Path(__file__).resolve().parent.parent
        os.chdir(src_dir)
        if not (src_dir / "core" / "kernel.asm").exists():
            messagebox.showerror("Error", 
                "Could not find kernel sources (expected src/core/kernel.asm).")
            sys.exit(1)
        
        # Variables
        self.build_running = False
        self.qemu_process = None
        self.animation_running = False
        
        # Setup UI
        self.setup_modern_ui()
        self.check_dependencies()
        self.start_idle_animation()
    
    def setup_modern_ui(self):
        """Setup the modern UI with animations"""
        # Main container with padding
        main_container = tk.Frame(self.root, bg=self.colors['bg'])
        main_container.pack(fill=tk.BOTH, expand=True, padx=20, pady=20)
        
        # Header with gradient effect
        self.create_header(main_container)
        
        # Content area
        content_frame = tk.Frame(main_container, bg=self.colors['bg'])
        content_frame.pack(fill=tk.BOTH, expand=True, pady=(20, 0))
        
        # Left panel - Controls
        left_panel = tk.Frame(content_frame, bg=self.colors['bg'])
        left_panel.pack(side=tk.LEFT, fill=tk.Y, padx=(0, 20))
        
        # Right panel - Logs
        right_panel = tk.Frame(content_frame, bg=self.colors['bg'])
        right_panel.pack(side=tk.RIGHT, fill=tk.BOTH, expand=True)
        
        # Setup panels
        self.create_control_panel(left_panel)
        self.create_log_panel(right_panel)
    
    def create_header(self, parent):
        """Create animated header"""
        header_frame = tk.Frame(parent, bg=self.colors['surface'], height=100)
        header_frame.pack(fill=tk.X, pady=(0, 20))
        header_frame.pack_propagate(False)
        
        # Add subtle border
        border_frame = tk.Frame(header_frame, bg=self.colors['primary'], height=3)
        border_frame.pack(fill=tk.X)
        
        # Title with animation
        title_container = tk.Frame(header_frame, bg=self.colors['surface'])
        title_container.pack(expand=True)
        
        self.title_label = tk.Label(title_container, 
                       text="🍿 Popcorn Kernel Builder",
                       font=("Arial", 28, "bold"),
                       bg=self.colors['surface'],
                       fg=self.colors['text'])
        self.title_label.pack(pady=(10, 5))
        
        self.subtitle_label = tk.Label(title_container,
                          text="x86-64 Long Mode Edition",
                          font=("Arial", 12),
                          bg=self.colors['surface'],
                          fg=self.colors['text_secondary'])
        self.subtitle_label.pack()
        
        # Status indicator
        self.status_frame = tk.Frame(title_container, bg=self.colors['surface'])
        self.status_frame.pack(pady=(5, 10))
        
        self.status_indicator = tk.Label(self.status_frame, text="●",
                                       font=("Arial", 12),
                                       bg=self.colors['surface'],
                                       fg=self.colors['success'])
        self.status_indicator.pack(side=tk.LEFT)
        
        self.status_label = tk.Label(self.status_frame, text="Ready",
                       font=("Arial", 10),
                       bg=self.colors['surface'],
                       fg=self.colors['text_secondary'])
        self.status_label.pack(side=tk.LEFT, padx=(5, 0))
    
    def create_control_panel(self, parent):
        """Create modern control panel"""
        # Automation section
        auto_frame = self.create_card(parent, "🚀 Automation")
        
        self.auto_btn = self.create_modern_button(
            auto_frame, 
            "Full Automation\n(Build & Run)",
            self.full_automation,
            self.colors['primary'],
            self.colors['primary_hover'],
            height=3
        )
        self.auto_btn.pack(fill=tk.X, pady=10)
        
        tk.Label(auto_frame, text="One-click: Clean → Build → ISO → Run",
            font=("Arial", 9),
            bg=self.colors['surface'],
            fg=self.colors['text_secondary']).pack()
        
        # Manual section
        manual_frame = self.create_card(parent, "🔧 Manual Mode")
        
        # Manual buttons grid
        btn_frame = tk.Frame(manual_frame, bg=self.colors['surface'])
        btn_frame.pack(fill=tk.X, pady=10)
        
        self.build_verbose_btn = self.create_modern_button(
            btn_frame,
            "🔨 Build Kernel\n(Verbose)",
            self.build_verbose,
            self.colors['secondary'],
            self.colors['primary'],
            width=20,
            height=2
        )
        self.build_verbose_btn.grid(row=0, column=0, padx=5, pady=5, sticky="ew")
        
        self.build_iso_btn = self.create_modern_button(
            btn_frame,
            "📀 Build + ISO\n(Quick)",
            self.build_and_iso,
            self.colors['accent'],
            self.colors['primary'],
            width=20,
            height=2
        )
        self.build_iso_btn.grid(row=0, column=1, padx=5, pady=5, sticky="ew")
        
        self.run_qemu_btn = self.create_modern_button(
            btn_frame,
            "▶️ Run QEMU\n(ISO Required)",
            self.run_qemu,
            self.colors['warning'],
            self.colors['primary'],
            width=20,
            height=2
        )
        self.run_qemu_btn.grid(row=1, column=0, padx=5, pady=5, sticky="ew")
        
        self.stop_qemu_btn = self.create_modern_button(
            btn_frame,
            "⏹️ Stop QEMU",
            self.stop_qemu,
            self.colors['error'],
            self.colors['primary'],
            width=20,
            height=2,
            state=tk.DISABLED
        )
        self.stop_qemu_btn.grid(row=1, column=1, padx=5, pady=5, sticky="ew")
        
        btn_frame.columnconfigure(0, weight=1)
        btn_frame.columnconfigure(1, weight=1)
        
        # Utilities
        util_frame = self.create_card(parent, "🛠️ Utilities")
        
        util_btn_frame = tk.Frame(util_frame, bg=self.colors['surface'])
        util_btn_frame.pack(fill=tk.X, pady=10)
        
        clean_btn = self.create_modern_button(
            util_btn_frame,
            "🧹 Clean Build Files",
            self.clean_build,
            self.colors['text_secondary'],
            self.colors['primary'],
            width=15
        )
        clean_btn.pack(side=tk.LEFT, padx=5)
        
        settings_btn = self.create_modern_button(
            util_btn_frame,
            "⚙️ Settings",
            self.show_settings,
            self.colors['text_secondary'],
            self.colors['primary'],
            width=15
        )
        settings_btn.pack(side=tk.LEFT, padx=5)
    
    def create_log_panel(self, parent):
        """Create modern log panel"""
        log_frame = self.create_card(parent, "📊 Build Output & Logs")
        
        # Log controls
        log_controls = tk.Frame(log_frame, bg=self.colors['surface'])
        log_controls.pack(fill=tk.X, pady=(0, 10))
        
        clear_log_btn = self.create_modern_button(
            log_controls,
            "🗑️ Clear",
            self.clear_logs,
            self.colors['text_secondary'],
            self.colors['primary'],
            width=8
        )
        clear_log_btn.pack(side=tk.LEFT)
        
        save_log_btn = self.create_modern_button(
            log_controls,
            "💾 Save Log",
            self.save_logs,
            self.colors['text_secondary'],
            self.colors['primary'],
            width=8
        )
        save_log_btn.pack(side=tk.LEFT, padx=(5, 0))
        
        # Log text area with modern styling
        self.log_text = scrolledtext.ScrolledText(
            log_frame,
            font=("JetBrains Mono", 10),
            bg=self.colors['bg'],
            fg=self.colors['text'],
            wrap=tk.WORD,
            relief=tk.FLAT,
            borderwidth=0,
            insertbackground=self.colors['primary']
        )
        self.log_text.pack(fill=tk.BOTH, expand=True, pady=(0, 10))
        
        # Configure text tags for colors
        self.setup_log_colors()
        
        # Progress bar with modern styling
        style = ttk.Style()
        style.theme_use('clam')
        style.configure("Modern.Horizontal.TProgressbar",
                       background=self.colors['primary'],
                       troughcolor=self.colors['surface'],
                       borderwidth=0,
                       lightcolor=self.colors['primary'],
                       darkcolor=self.colors['primary'])
        
        self.progress = ttk.Progressbar(log_frame, 
                                       style="Modern.Horizontal.TProgressbar",
                                       mode='indeterminate')
        self.progress.pack(fill=tk.X)
    
    def create_card(self, parent, title):
        """Create a modern card container"""
        card = tk.Frame(parent, bg=self.colors['surface'], relief=tk.FLAT)
        card.pack(fill=tk.X, pady=(0, 15))
        
        # Card header
        header = tk.Frame(card, bg=self.colors['surface'])
        header.pack(fill=tk.X, padx=15, pady=(15, 10))
        
        title_label = tk.Label(header, text=title,
                      font=("Arial", 12, "bold"),
                      bg=self.colors['surface'],
                      fg=self.colors['text'])
        title_label.pack(anchor=tk.W)
        
        # Card content
        content = tk.Frame(card, bg=self.colors['surface'])
        content.pack(fill=tk.BOTH, expand=True, padx=15, pady=(0, 15))
        
        return content
    
    def create_modern_button(self, parent, text, command, bg_color, hover_color, 
                            width=None, height=None, state=tk.NORMAL):
        """Create a modern animated button"""
        btn = tk.Button(parent,
                   text=text,
                   command=command,
                   bg=bg_color,
                   fg=self.colors['text'],
                   font=("Arial", 10, "bold"),
                   relief=tk.FLAT,
                   borderwidth=0,
                   cursor="hand2",
                   state=state,
                   width=width,
                   height=height)
        
        # Add hover effects
        def on_enter(e):
            if btn['state'] != tk.DISABLED:
                btn.config(bg=hover_color)
                self.animate_button_scale(btn, 1.05)
        
        def on_leave(e):
            if btn['state'] != tk.DISABLED:
                btn.config(bg=bg_color)
                self.animate_button_scale(btn, 1.0)
        
        btn.bind("<Enter>", on_enter)
        btn.bind("<Leave>", on_leave)
        
        return btn
    
    def animate_button_scale(self, widget, scale):
        """Animate button scale"""
        def animate():
            if hasattr(widget, '_animation_id'):
                self.root.after_cancel(widget._animation_id)
            
            current_scale = getattr(widget, '_current_scale', 1.0)
            if abs(current_scale - scale) < 0.01:
                widget._current_scale = scale
                return
            
            # Smooth scaling animation
            diff = (scale - current_scale) * 0.2
            new_scale = current_scale + diff
            
            # Apply scaling by adjusting font size
            font_size = int(10 * new_scale)
            widget.config(font=("Arial", font_size, "bold"))
            
            widget._current_scale = new_scale
            widget._animation_id = self.root.after(16, animate)  # ~60fps
        
        animate()
    
    def setup_log_colors(self):
        """Setup color tags for log messages"""
        colors = {
            "timestamp": self.colors['text_secondary'],
            "INFO": self.colors['primary'],
            "SUCCESS": self.colors['success'],
            "WARNING": self.colors['warning'],
            "ERROR": self.colors['error'],
            "COMMAND": self.colors['secondary']
        }
        
        for tag, color in colors.items():
            self.log_text.tag_config(tag, foreground=color, font=("JetBrains Mono", 10, "bold"))
    
    def start_idle_animation(self):
        """Start subtle idle animations"""
        def animate_title():
            if not self.build_running:
                # Subtle color pulse for title
                colors = [self.colors['primary'], self.colors['secondary'], self.colors['accent']]
                current_color = getattr(self, '_title_color_index', 0)
                
                self.title_label.config(fg=colors[current_color])
                self._title_color_index = (current_color + 1) % len(colors)
                
                self.root.after(3000, animate_title)
            else:
                self.root.after(1000, animate_title)
        
        animate_title()
    
    def animate_status_indicator(self, color, text):
        """Animate status indicator"""
        # Use Tk's `after` to modify widgets on the main thread instead
        pulses = 3

        def do_pulse(count):
            if count <= 0:
                # restore neutral color after pulses
                self.status_indicator.config(fg=self.colors['text_secondary'])
                return

            # set to active color
            self.status_indicator.config(fg=color)

            # schedule revert to secondary color after 100ms, then continue pulses
            def revert_and_continue():
                self.status_indicator.config(fg=self.colors['text_secondary'])
                # schedule next pulse after another 100ms
                self.root.after(100, lambda: do_pulse(count - 1))

            self.root.after(100, revert_and_continue)

        # Ensure the status label update is done on the main thread
        self.root.after(0, lambda: self.status_label.config(text=text, fg=color))
        # start pulsing on the main thread
        self.root.after(0, lambda: do_pulse(pulses))

    def call_on_main(self, func, *args, **kwargs):
        """Schedule `func` to run on the Tk main thread."""
        try:
            self.root.after(0, lambda: func(*args, **kwargs))
        except Exception:
            # In case root isn't available, call directly as fallback
            try:
                func(*args, **kwargs)
            except Exception:
                pass

    
    def check_dependencies(self):
        """Check for required build tools"""
        self.log("🔍 Checking dependencies...")
        
        required = {
            'nasm': 'NASM assembler',
            'gcc': 'GCC compiler',
            'ld': 'GNU linker',
            'qemu-system-x86_64': 'QEMU (64-bit)',
            'grub2-mkrescue': 'GRUB2 mkrescue'
        }
        
        missing = []
        for cmd, desc in required.items():
            if subprocess.run(['which', cmd], capture_output=True).returncode != 0:
                missing.append(f"  • {desc} ({cmd})")
        
        if missing:
            msg = "❌ Missing dependencies:\n" + "\n".join(missing)
            msg += "\n\n💡 Install with:\nsudo dnf install nasm gcc qemu-system-x86 grub2-tools-extra grub2-pc-modules xorriso mtools"
            self.log(msg, "ERROR")
            self.animate_status_indicator(self.colors['error'], "Missing dependencies")
            # showwarning runs on main thread (this function is called during init on main)
            # Schedule the dialog to appear after mainloop starts to avoid
            # Tcl errors when called too early (some platforms require an
            # active event loop before grab can be used).
            try:
                self.root.after(500, lambda: messagebox.showwarning("Missing Dependencies", msg))
            except Exception:
                # Fallback to printing if scheduling fails
                print(msg)
        else:
            self.log("✅ All dependencies found", "SUCCESS")
            self.animate_status_indicator(self.colors['success'], "Ready")
    
    def log(self, message, level="INFO"):
        """Add message to log with color coding"""
        timestamp = subprocess.run(['date', '+%H:%M:%S'], capture_output=True, text=True).stdout.strip()

        def _append():
            try:
                self.log_text.insert(tk.END, f"[{timestamp}] ", "timestamp")
                self.log_text.insert(tk.END, f"[{level}] ", level)
                self.log_text.insert(tk.END, f"{message}\n")
                self.log_text.see(tk.END)
            except Exception:
                # If GUI is not available or destroyed, ignore
                pass

        # Always schedule append on the main thread to avoid Tk race conditions
        self.call_on_main(_append)
    
    def clear_logs(self):
        """Clear log output"""
        self.log_text.delete(1.0, tk.END)
        self.log("📝 Log cleared", "INFO")
    
    def save_logs(self):
        """Save logs to file"""
        try:
            with open('build_log.txt', 'w') as f:
                f.write(self.log_text.get(1.0, tk.END))
            self.log("💾 Log saved to build_log.txt", "SUCCESS")
        except Exception as e:
            self.log(f"❌ Failed to save log: {e}", "ERROR")
    
    def show_settings(self):
        """Show settings dialog"""
        settings_window = tk.Toplevel(self.root)
        settings_window.title("Settings")
        settings_window.geometry("400x300")
        settings_window.configure(bg=self.colors['surface'])
        settings_window.resizable(False, False)
        
        # Center the window
        settings_window.transient(self.root)
        settings_window.grab_set()
        
        tk.Label(settings_window, text="⚙️ Build Settings",
            font=("Arial", 16, "bold"),
            bg=self.colors['surface'],
            fg=self.colors['text']).pack(pady=20)
        
        tk.Label(settings_window, text="Settings panel coming soon!",
            font=("Arial", 12),
            bg=self.colors['surface'],
            fg=self.colors['text_secondary']).pack(pady=50)
        
        tk.Button(settings_window, text="Close",
            command=settings_window.destroy,
            bg=self.colors['primary'],
            fg=self.colors['text'],
            font=("Arial", 10, "bold"),
            relief=tk.FLAT,
            borderwidth=0,
            cursor="hand2",
            width=10).pack(pady=20)
    
    def disable_buttons(self):
        """Disable build buttons during operation"""
        def _do_disable():
            self.build_running = True
            try:
                self.build_verbose_btn.config(state=tk.DISABLED)
                self.build_iso_btn.config(state=tk.DISABLED)
                self.run_qemu_btn.config(state=tk.DISABLED)
                self.progress.start()
            except Exception:
                pass

        self.call_on_main(_do_disable)
    
    def enable_buttons(self):
        """Enable build buttons after operation"""
        def _do_enable():
            self.build_running = False
            try:
                self.build_verbose_btn.config(state=tk.NORMAL)
                self.build_iso_btn.config(state=tk.NORMAL)
                self.run_qemu_btn.config(state=tk.NORMAL)
                self.progress.stop()
            except Exception:
                pass

        self.call_on_main(_do_enable)
    
    def run_command(self, cmd, verbose=False, log_file=None):
        """Run a shell command and capture output"""
        self.log(f"⚡ Running: {' '.join(cmd)}", "COMMAND")
        
        try:
            if verbose:
                # Real-time output
                process = subprocess.Popen(cmd, stdout=subprocess.PIPE, 
                                         stderr=subprocess.STDOUT, text=True)
                output_lines = []
                for line in process.stdout:
                    line = line.rstrip()
                    self.log(line, "INFO")
                    output_lines.append(line)
                
                process.wait()
                
                if log_file and process.returncode != 0:
                    with open(log_file, 'w') as f:
                        f.write('\n'.join(output_lines))
                    self.log(f"📄 Error log saved to: {log_file}", "WARNING")
                
                return process.returncode == 0
            else:
                # Quiet mode
                result = subprocess.run(cmd, capture_output=True, text=True)
                if result.returncode != 0:
                    self.log(result.stderr, "ERROR")
                    if log_file:
                        with open(log_file, 'w') as f:
                            f.write(result.stdout + '\n' + result.stderr)
                return result.returncode == 0
        
        except Exception as e:
            self.log(f"💥 Command failed: {e}", "ERROR")
            return False
    
    def clean_build(self):
        """Clean build artifacts"""
        self.log("🧹 Cleaning build files...")
        self.animate_status_indicator(self.colors['warning'], "Cleaning...")
        
        for path in ['obj', 'kernel', 'popcorn.iso', 'isodir', 'build_errors.log']:
            if Path(path).exists():
                subprocess.run(['rm', '-rf', path])
                self.log(f"🗑️ Removed: {path}")
        
        self.log("✅ Clean complete", "SUCCESS")
        self.animate_status_indicator(self.colors['success'], "Ready")
    
    def build_verbose(self):
        """Build kernel with verbose output and error logs"""
        def build():
            self.disable_buttons()
            self.animate_status_indicator(self.colors['primary'], "Building (verbose)...")
            self.log("=" * 60)
            self.log("🔨 VERBOSE BUILD MODE - Full output enabled", "INFO")
            self.log("=" * 60)
            
            # Clean first
            self.run_command(['rm', '-rf', 'obj'])
            self.run_command(['mkdir', '-p', 'obj'])
            
            success = True
            error_log = 'build_errors.log'
            
            # Compile assembly files
            asm_files = [
                ('core/kernel.asm', 'obj/kasm.o'), 
                ('core/idt.asm', 'obj/idt.o'),
                ('core/context_switch.asm', 'obj/context_switch.o')
            ]
            for src, obj in asm_files:
                if not self.run_command(['nasm', '-f', 'elf64', src, '-o', obj], 
                                       verbose=True, log_file=error_log):
                    success = False
                    break
            
            # Compile C files
            if success:
                c_files = [
                    ('core/kernel.c', 'obj/kc.o'),
                    ('core/console.c', 'obj/console.o'),
                    ('core/utils.c', 'obj/utils.o'),
                    ('core/pop_module.c', 'obj/pop_module.o'),
                    ('pops/shimjapii_pop.c', 'obj/shimjapii_pop.o'),
                    ('pops/spinner_pop.c', 'obj/spinner_pop.o'),
                    ('pops/uptime_pop.c', 'obj/uptime_pop.o'),
                    ('pops/halt_pop.c', 'obj/halt_pop.o'),
                    ('pops/filesystem_pop.c', 'obj/filesystem_pop.o'),
                    ('core/multiboot2.c', 'obj/multiboot2.o'),
                    ('pops/sysinfo_pop.c', 'obj/sysinfo_pop.o'),
                    ('pops/memory_pop.c', 'obj/memory_pop.o'),
                    ('pops/cpu_pop.c', 'obj/cpu_pop.o'),
                    ('pops/dolphin_pop.c', 'obj/dolphin_pop.o'),
                    ('core/timer.c', 'obj/timer.o'),
                    ('core/scheduler.c', 'obj/scheduler.o'),
                    ('core/memory.c', 'obj/memory.o'),
                    ('core/vmm.c', 'obj/vmm.o'),
                    ('core/init.c', 'obj/init.o'),
                    ('core/syscall.c', 'obj/syscall.o')
                ]
                
                for src, obj in c_files:
                    if not self.run_command(['gcc', '-m64', '-c', src, '-o', obj,
                                           '-Wall', '-Wextra', '-fno-stack-protector',
                                           '-mcmodel=large', '-mno-red-zone'],
                                          verbose=True, log_file=error_log):
                        success = False
                        break
            
            # Link
            if success:
                obj_files = ['obj/kasm.o', 'obj/kc.o', 'obj/console.o', 'obj/utils.o',
                           'obj/pop_module.o', 'obj/shimjapii_pop.o', 'obj/idt.o',
                           'obj/context_switch.o', 'obj/spinner_pop.o', 'obj/uptime_pop.o', 
                           'obj/halt_pop.o', 'obj/filesystem_pop.o', 'obj/multiboot2.o',
                           'obj/sysinfo_pop.o', 'obj/memory_pop.o', 'obj/cpu_pop.o', 
                           'obj/dolphin_pop.o', 'obj/timer.o', 'obj/scheduler.o', 
                           'obj/memory.o', 'obj/vmm.o', 'obj/init.o', 'obj/syscall.o']
                
                success = self.run_command(['ld', '-m', 'elf_x86_64', '-T', 'link.ld',
                                          '-o', 'kernel'] + obj_files,
                                         verbose=True, log_file=error_log)
            
            if success:
                self.log("=" * 60)
                self.log("🎉 BUILD SUCCESSFUL!", "SUCCESS")
                self.log("=" * 60)
                self.animate_status_indicator(self.colors['success'], "Build successful!")
                # Show dialog on main thread
                self.call_on_main(messagebox.showinfo, "Success", "✅ Kernel built successfully!\n\nNext: Build ISO or run in QEMU")
            else:
                self.log("=" * 60)
                self.log("💥 BUILD FAILED - Check error log", "ERROR")
                self.log("=" * 60)
                self.animate_status_indicator(self.colors['error'], "Build failed")
                self.call_on_main(messagebox.showerror, "Build Failed",
                                  f"❌ Build failed. Check the output above.\nErrors saved to: {error_log}")
            
            self.enable_buttons()
        
        threading.Thread(target=build, daemon=True).start()
    
    def build_and_iso(self):
        """Build kernel and create ISO (non-verbose)"""
        def build():
            self.disable_buttons()
            self.animate_status_indicator(self.colors['secondary'], "Building + ISO...")
            self.log("🔨 Building kernel (quick mode)...")
            
            # Build kernel with all source files
            success = self.run_command(['bash', '-c', 
                'mkdir -p obj && ' +
                'nasm -f elf64 core/kernel.asm -o obj/kasm.o && ' +
                'nasm -f elf64 core/idt.asm -o obj/idt.o && ' +
                'nasm -f elf64 core/context_switch.asm -o obj/context_switch.o && ' +
                'gcc -m64 -c core/kernel.c -o obj/kc.o -Wall -Wextra -fno-stack-protector -mcmodel=large -mno-red-zone && ' +
                'gcc -m64 -c core/console.c -o obj/console.o -Wall -Wextra -fno-stack-protector -mcmodel=large -mno-red-zone && ' +
                'gcc -m64 -c core/utils.c -o obj/utils.o -Wall -Wextra -fno-stack-protector -mcmodel=large -mno-red-zone && ' +
                'gcc -m64 -c core/pop_module.c -o obj/pop_module.o -Wall -Wextra -fno-stack-protector -mcmodel=large -mno-red-zone && ' +
                'gcc -m64 -c pops/shimjapii_pop.c -o obj/shimjapii_pop.o -Wall -Wextra -fno-stack-protector -mcmodel=large -mno-red-zone && ' +
                'gcc -m64 -c pops/spinner_pop.c -o obj/spinner_pop.o -Wall -Wextra -fno-stack-protector -mcmodel=large -mno-red-zone && ' +
                'gcc -m64 -c pops/uptime_pop.c -o obj/uptime_pop.o -Wall -Wextra -fno-stack-protector -mcmodel=large -mno-red-zone && ' +
                'gcc -m64 -c pops/halt_pop.c -o obj/halt_pop.o -Wall -Wextra -fno-stack-protector -mcmodel=large -mno-red-zone && ' +
                'gcc -m64 -c pops/filesystem_pop.c -o obj/filesystem_pop.o -Wall -Wextra -fno-stack-protector -mcmodel=large -mno-red-zone && ' +
                'gcc -m64 -c core/multiboot2.c -o obj/multiboot2.o -Wall -Wextra -fno-stack-protector -mcmodel=large -mno-red-zone && ' +
                'gcc -m64 -c pops/sysinfo_pop.c -o obj/sysinfo_pop.o -Wall -Wextra -fno-stack-protector -mcmodel=large -mno-red-zone && ' +
                'gcc -m64 -c pops/memory_pop.c -o obj/memory_pop.o -Wall -Wextra -fno-stack-protector -mcmodel=large -mno-red-zone && ' +
                'gcc -m64 -c pops/cpu_pop.c -o obj/cpu_pop.o -Wall -Wextra -fno-stack-protector -mcmodel=large -mno-red-zone && ' +
                'gcc -m64 -c pops/dolphin_pop.c -o obj/dolphin_pop.o -Wall -Wextra -fno-stack-protector -mcmodel=large -mno-red-zone && ' +
                'gcc -m64 -c core/timer.c -o obj/timer.o -Wall -Wextra -fno-stack-protector -mcmodel=large -mno-red-zone && ' +
                'gcc -m64 -c core/scheduler.c -o obj/scheduler.o -Wall -Wextra -fno-stack-protector -mcmodel=large -mno-red-zone && ' +
                'gcc -m64 -c core/memory.c -o obj/memory.o -Wall -Wextra -fno-stack-protector -mcmodel=large -mno-red-zone && ' +
                'gcc -m64 -c core/vmm.c -o obj/vmm.o -Wall -Wextra -fno-stack-protector -mcmodel=large -mno-red-zone && ' +
                'gcc -m64 -c core/init.c -o obj/init.o -Wall -Wextra -fno-stack-protector -mcmodel=large -mno-red-zone && ' +
                'gcc -m64 -c core/syscall.c -o obj/syscall.o -Wall -Wextra -fno-stack-protector -mcmodel=large -mno-red-zone && ' +
                'ld -m elf_x86_64 -T link.ld -o kernel obj/kasm.o obj/kc.o obj/console.o obj/utils.o obj/pop_module.o obj/shimjapii_pop.o obj/idt.o obj/context_switch.o obj/spinner_pop.o obj/uptime_pop.o obj/halt_pop.o obj/filesystem_pop.o obj/multiboot2.o obj/sysinfo_pop.o obj/memory_pop.o obj/cpu_pop.o obj/dolphin_pop.o obj/timer.o obj/scheduler.o obj/memory.o obj/vmm.o obj/init.o obj/syscall.o'
            ])
            
            if success:
                self.log("✅ Kernel built successfully", "SUCCESS")
                self.log("📀 Creating bootable ISO...")
                
                # Create ISO structure
                iso_success = self.run_command(['bash', '-c',
                    'rm -rf isodir && ' +
                    'mkdir -p isodir/boot/grub && ' +
                    'cp kernel isodir/boot/kernel && ' +
                    'echo "set timeout=3" > isodir/boot/grub/grub.cfg && ' +
                    'echo "set default=0" >> isodir/boot/grub/grub.cfg && ' +
                    'echo "" >> isodir/boot/grub/grub.cfg && ' +
                    'echo "menuentry \\"Popcorn Kernel x64\\" {" >> isodir/boot/grub/grub.cfg && ' +
                    'echo "    echo \\"Loading Popcorn kernel...\\"" >> isodir/boot/grub/grub.cfg && ' +
                    'echo "    multiboot2 /boot/kernel" >> isodir/boot/grub/grub.cfg && ' +
                    'echo "    echo \\"Booting kernel...\\"" >> isodir/boot/grub/grub.cfg && ' +
                    'echo "    boot" >> isodir/boot/grub/grub.cfg && ' +
                    'echo "}" >> isodir/boot/grub/grub.cfg && ' +
                    'grub2-mkrescue -o popcorn.iso isodir 2>&1 || grub-mkrescue -o popcorn.iso isodir 2>&1 && ' +
                    'rm -rf isodir'
                ])
                
                if iso_success:
                    self.log("✅ ISO created: popcorn.iso", "SUCCESS")
                    self.animate_status_indicator(self.colors['success'], "Build complete!")
                    self.call_on_main(messagebox.showinfo, "Success",
                                      "🎉 Kernel and ISO created successfully!\n\n📀 popcorn.iso is ready to boot.")
                else:
                    self.log("❌ ISO creation failed", "ERROR")
                    self.animate_status_indicator(self.colors['error'], "ISO creation failed")
            else:
                self.log("❌ Build failed", "ERROR")
                self.animate_status_indicator(self.colors['error'], "Build failed")
                self.call_on_main(messagebox.showerror, "Build Failed", "❌ Build failed. Check the output above.")
            
            self.enable_buttons()
        
        threading.Thread(target=build, daemon=True).start()
    
    def run_qemu(self):
        """Run the ISO in QEMU"""
        if not Path('popcorn.iso').exists():
            self.call_on_main(messagebox.showerror, "ISO Not Found",
                              "❌ popcorn.iso not found!\n\nPlease build the kernel and create ISO first.")
            return
        
        def run():
            self.disable_buttons()
            # ensure button state changes happen on main thread
            self.call_on_main(lambda: self.stop_qemu_btn.config(state=tk.NORMAL))
            self.animate_status_indicator(self.colors['warning'], "Running QEMU...")
            self.log("🚀 Starting QEMU with popcorn.iso...")
            self.log("💡 Press Ctrl+C in terminal or click 'Stop QEMU' to exit")
            
            try:
                self.qemu_process = subprocess.Popen([
                    'qemu-system-x86_64',
                    '-cdrom', 'popcorn.iso',
                    '-cpu', 'qemu64',
                    '-m', '256',
                    '-smp', '1',
                    '-serial', 'stdio'
                ])
                
                self.qemu_process.wait()
                self.log("✅ QEMU session ended", "INFO")
                
            except Exception as e:
                self.log(f"💥 QEMU error: {e}", "ERROR")
            
            finally:
                self.qemu_process = None
                self.call_on_main(lambda: self.stop_qemu_btn.config(state=tk.DISABLED))
                self.animate_status_indicator(self.colors['success'], "Ready")
                self.enable_buttons()
        
        threading.Thread(target=run, daemon=True).start()
    
    def stop_qemu(self):
        """Stop QEMU if running"""
        if self.qemu_process:
            self.log("⏹️ Stopping QEMU...", "WARNING")
            self.qemu_process.terminate()
            self.qemu_process = None
            self.stop_qemu_btn.config(state=tk.DISABLED)
    
    def full_automation(self):
        """Full automation: clean, build, create ISO, and run"""
        def automate():
            self.disable_buttons()
            self.animate_status_indicator(self.colors['primary'], "Full automation...")
            
            self.log("=" * 60)
            self.log("🚀 FULL AUTOMATION MODE", "SUCCESS")
            self.log("=" * 60)
            
            # Step 1: Clean
            self.log("📋 Step 1/4: Cleaning...")
            self.clean_build()
            
            # Step 2: Build kernel
            self.log("📋 Step 2/4: Building kernel...")
            success = self.run_command(['bash', '-c', 
                'mkdir -p obj && ' +
                'nasm -f elf64 core/kernel.asm -o obj/kasm.o && ' +
                'nasm -f elf64 core/idt.asm -o obj/idt.o && ' +
                'nasm -f elf64 core/context_switch.asm -o obj/context_switch.o && ' +
                'gcc -m64 -c core/*.c -Wall -Wextra -fno-stack-protector -mcmodel=large -mno-red-zone && ' +
                'gcc -m64 -c pops/*.c -Wall -Wextra -fno-stack-protector -mcmodel=large -mno-red-zone && ' +
                'mv *.o obj/ 2>/dev/null; ' +
                'ld -m elf_x86_64 -T link.ld -o kernel obj/*.o'
            ])
            
            if not success:
                self.log("❌ Automation failed at build step", "ERROR")
                self.enable_buttons()
                self.call_on_main(messagebox.showerror, "Automation Failed", "❌ Build failed. Check logs.")
                return
            
            self.log("✅ Build successful", "SUCCESS")
            
            # Step 3: Create ISO
            self.log("📋 Step 3/4: Creating ISO...")
            iso_success = self.run_command(['bash', '-c',
                'rm -rf isodir && ' +
                'mkdir -p isodir/boot/grub && ' +
                'cp kernel isodir/boot/kernel && ' +
                'echo "set timeout=3" > isodir/boot/grub/grub.cfg && ' +
                'echo "set default=0" >> isodir/boot/grub/grub.cfg && ' +
                'echo "" >> isodir/boot/grub/grub.cfg && ' +
                'echo "menuentry \\"Popcorn Kernel x64\\" {" >> isodir/boot/grub/grub.cfg && ' +
                'echo "    echo \\"Loading Popcorn kernel...\\"" >> isodir/boot/grub/grub.cfg && ' +
                'echo "    multiboot2 /boot/kernel" >> isodir/boot/grub/grub.cfg && ' +
                'echo "    echo \\"Booting kernel...\\"" >> isodir/boot/grub/grub.cfg && ' +
                'echo "    boot" >> isodir/boot/grub/grub.cfg && ' +
                'echo "}" >> isodir/boot/grub/grub.cfg && ' +
                'grub2-mkrescue -o popcorn.iso isodir 2>&1 || grub-mkrescue -o popcorn.iso isodir 2>&1 && ' +
                'rm -rf isodir'
            ])
            
            if not iso_success:
                self.log("❌ Automation failed at ISO creation", "ERROR")
                self.enable_buttons()
                self.call_on_main(messagebox.showerror, "Automation Failed", "❌ ISO creation failed. Check logs.")
                return
            
            self.log("✅ ISO created", "SUCCESS")
            
            # Step 4: Run QEMU
            self.log("📋 Step 4/4: Launching QEMU...")
            self.log("=" * 60)
            self.log("🎉 AUTOMATION COMPLETE - Starting kernel...", "SUCCESS")
            self.log("=" * 60)
            
            self.enable_buttons()
            self.run_qemu()
        
        threading.Thread(target=automate, daemon=True).start()

def main():
    root = tk.Tk()
    app = ModernPopcornBuilder(root)
    root.mainloop()

if __name__ == "__main__":
    main()

