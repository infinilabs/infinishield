package main

import (
	"encoding/json"
	"fmt"
	"io"
	"log"
	"net/http"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"time"
)

const (
	port       = ":1983"
	uploadDir  = "webapp/uploads"
	outputDir  = "webapp/outputs"
	logDir     = "webapp/logs"
	binaryName = "infinishield"
)

var eventLog *log.Logger

func main() {
	os.MkdirAll(uploadDir, 0755)
	os.MkdirAll(outputDir, 0755)
	os.MkdirAll(logDir, 0755)

	// Setup event log file
	logFile, err := os.OpenFile(
		filepath.Join(logDir, time.Now().Format("2006-01-02")+".log"),
		os.O_CREATE|os.O_WRONLY|os.O_APPEND, 0644,
	)
	if err != nil {
		log.Fatalf("Failed to open log file: %v", err)
	}
	defer logFile.Close()
	eventLog = log.New(logFile, "", 0)

	binary := findBinary()
	if binary == "" {
		log.Fatal("infinishield release binary not found at target/release/infinishield. Run 'make release' or 'make release-video' first.")
	}
	log.Printf("Using binary: %s", binary)
	logEvent("SERVER_START", nil, map[string]string{"binary": binary})

	http.HandleFunc("/", serveIndex)
	http.Handle("/static/", http.StripPrefix("/static/", http.FileServer(http.Dir("webapp/static"))))
	http.Handle("/uploads/", http.StripPrefix("/uploads/", http.FileServer(http.Dir(uploadDir))))
	http.Handle("/outputs/", http.StripPrefix("/outputs/", http.FileServer(http.Dir(outputDir))))

	http.HandleFunc("/api/upload", handleUpload)
	http.HandleFunc("/api/dryrun", makeDryRunHandler(binary))
	http.HandleFunc("/api/embed", makeEmbedHandler(binary))
	http.HandleFunc("/api/verify", makeVerifyHandler(binary))

	log.Printf("infinishield webapp running on http://localhost%s", port)
	log.Fatal(http.ListenAndServe(port, nil))
}

// ── Structured Event Logging ─────────────────────────────────────────────

func logEvent(event string, r *http.Request, data map[string]string) {
	entry := map[string]interface{}{
		"time":  time.Now().UTC().Format(time.RFC3339),
		"event": event,
	}

	if r != nil {
		entry["remote_addr"] = r.RemoteAddr
		entry["user_agent"] = r.UserAgent()
		entry["referer"] = r.Referer()
		entry["method"] = r.Method
		entry["path"] = r.URL.Path
	}

	for k, v := range data {
		entry[k] = v
	}

	jsonBytes, _ := json.Marshal(entry)
	eventLog.Println(string(jsonBytes))
}

func logEventR(event string, r *http.Request, data map[string]string) {
	logEvent(event, r, data)
}

// ── Binary Finder ────────────────────────────────────────────────────────

func findBinary() string {
	candidates := []string{
		"target/release/infinishield",
	}
	for _, c := range candidates {
		if _, err := os.Stat(c); err == nil {
			return c
		}
	}
	if p, err := exec.LookPath(binaryName); err == nil {
		return p
	}
	return ""
}

// ── Handlers ─────────────────────────────────────────────────────────────

func serveIndex(w http.ResponseWriter, r *http.Request) {
	if r.URL.Path != "/" {
		http.NotFound(w, r)
		return
	}
	logEventR("PAGE_VIEW", r, nil)
	http.ServeFile(w, r, "webapp/static/index.html")
}

type UploadResponse struct {
	Filename string `json:"filename"`
	Path     string `json:"path"`
	Size     int64  `json:"size"`
	Type     string `json:"type"`
}

func handleUpload(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodPost {
		http.Error(w, "POST only", http.StatusMethodNotAllowed)
		return
	}

	r.ParseMultipartForm(100 << 20)
	file, header, err := r.FormFile("file")
	if err != nil {
		logEventR("UPLOAD_ERROR", r, map[string]string{"error": err.Error()})
		jsonError(w, "No file uploaded", http.StatusBadRequest)
		return
	}
	defer file.Close()

	savePath := filepath.Join(uploadDir, header.Filename)
	dst, err := os.Create(savePath)
	if err != nil {
		logEventR("UPLOAD_ERROR", r, map[string]string{"error": err.Error()})
		jsonError(w, "Failed to save file", http.StatusInternalServerError)
		return
	}
	defer dst.Close()
	io.Copy(dst, file)

	ext := filepath.Ext(header.Filename)
	fileType := detectType(ext)

	logEventR("UPLOAD", r, map[string]string{
		"filename": header.Filename,
		"size":     fmt.Sprintf("%d", header.Size),
		"type":     fileType,
	})

	resp := UploadResponse{
		Filename: header.Filename,
		Path:     savePath,
		Size:     header.Size,
		Type:     fileType,
	}
	jsonResponse(w, resp)
}

func detectType(ext string) string {
	ext = strings.ToLower(ext)
	switch ext {
	case ".jpg", ".jpeg", ".png", ".webp", ".bmp", ".tiff", ".tif", ".gif":
		return "image"
	case ".svg":
		return "svg"
	case ".mp4", ".webm", ".mov", ".avi", ".mkv":
		return "video"
	default:
		return "unknown"
	}
}

type APIResponse struct {
	Success bool   `json:"success"`
	Output  string `json:"output"`
	Error   string `json:"error,omitempty"`
}

func makeDryRunHandler(binary string) http.HandlerFunc {
	return func(w http.ResponseWriter, r *http.Request) {
		if r.Method != http.MethodPost {
			http.Error(w, "POST only", http.StatusMethodNotAllowed)
			return
		}

		var req struct {
			InputPath string `json:"input_path"`
			Message   string `json:"message"`
			Password  string `json:"password"`
			Intensity string `json:"intensity"`
		}
		json.NewDecoder(r.Body).Decode(&req)

		start := time.Now()

		args := []string{"embed", "-i", req.InputPath, "-o", "/dev/null", "--dry-run"}
		if req.Message != "" {
			args = append(args, "-m", req.Message)
		}
		if req.Password != "" {
			args = append(args, "-p", req.Password)
		}
		if req.Intensity != "" && req.Intensity != "auto" {
			args = append(args, "--intensity", req.Intensity)
		}

		out, err := exec.Command(binary, args...).CombinedOutput()
		elapsed := time.Since(start)
		success := err == nil

		logEventR("DRYRUN", r, map[string]string{
			"input":     req.InputPath,
			"message":   req.Message,
			"intensity": req.Intensity,
			"success":   fmt.Sprintf("%v", success),
			"elapsed":   elapsed.String(),
		})

		if !success {
			jsonResponse(w, APIResponse{Success: false, Output: string(out), Error: err.Error()})
			return
		}
		jsonResponse(w, APIResponse{Success: true, Output: string(out)})
	}
}

func makeEmbedHandler(binary string) http.HandlerFunc {
	return func(w http.ResponseWriter, r *http.Request) {
		if r.Method != http.MethodPost {
			http.Error(w, "POST only", http.StatusMethodNotAllowed)
			return
		}

		var req struct {
			InputPath  string `json:"input_path"`
			Message    string `json:"message"`
			Password   string `json:"password"`
			Intensity  string `json:"intensity"`
			OutputName string `json:"output_name"`
		}
		json.NewDecoder(r.Body).Decode(&req)

		start := time.Now()
		outputPath := filepath.Join(outputDir, req.OutputName)
		args := []string{"embed", "-i", req.InputPath, "-o", outputPath}
		if req.Message != "" {
			args = append(args, "-m", req.Message)
		}
		if req.Password != "" {
			args = append(args, "-p", req.Password)
		}
		if req.Intensity != "" && req.Intensity != "auto" {
			args = append(args, "--intensity", req.Intensity)
		}

		out, err := exec.Command(binary, args...).CombinedOutput()
		embedElapsed := time.Since(start)

		resp := struct {
			Success    bool   `json:"success"`
			Output     string `json:"output"`
			Error      string `json:"error,omitempty"`
			OutputPath string `json:"output_path,omitempty"`
			OutputURL  string `json:"output_url,omitempty"`
		}{
			Success:    err == nil,
			Output:     string(out),
			OutputPath: outputPath,
			OutputURL:  "/outputs/" + req.OutputName,
		}
		if err != nil {
			resp.Error = err.Error()
		}

		// Auto-verify
		verifySuccess := false
		if err == nil {
			verifyArgs := []string{"verify", "-i", outputPath}
			if req.Password != "" {
				verifyArgs = append(verifyArgs, "-p", req.Password)
			}
			verifyOut, verifyErr := exec.Command(binary, verifyArgs...).CombinedOutput()
			verifySuccess = verifyErr == nil
			resp.Output += "\n--- Verification ---\n" + string(verifyOut)
		}

		totalElapsed := time.Since(start)

		logEventR("EMBED", r, map[string]string{
			"input":          req.InputPath,
			"output":         outputPath,
			"message":        req.Message,
			"intensity":      req.Intensity,
			"embed_success":  fmt.Sprintf("%v", err == nil),
			"verify_success": fmt.Sprintf("%v", verifySuccess),
			"embed_elapsed":  embedElapsed.String(),
			"total_elapsed":  totalElapsed.String(),
		})

		jsonResponse(w, resp)
	}
}

func makeVerifyHandler(binary string) http.HandlerFunc {
	return func(w http.ResponseWriter, r *http.Request) {
		if r.Method != http.MethodPost {
			http.Error(w, "POST only", http.StatusMethodNotAllowed)
			return
		}

		var req struct {
			InputPath string `json:"input_path"`
			Password  string `json:"password"`
		}
		json.NewDecoder(r.Body).Decode(&req)

		start := time.Now()

		args := []string{"verify", "-i", req.InputPath}
		if req.Password != "" {
			args = append(args, "-p", req.Password)
		}

		out, err := exec.Command(binary, args...).CombinedOutput()
		elapsed := time.Since(start)
		success := err == nil

		logEventR("VERIFY", r, map[string]string{
			"input":   req.InputPath,
			"success": fmt.Sprintf("%v", success),
			"elapsed": elapsed.String(),
		})

		if !success {
			jsonResponse(w, APIResponse{Success: false, Output: string(out), Error: err.Error()})
			return
		}
		jsonResponse(w, APIResponse{Success: true, Output: string(out)})
	}
}

// ── JSON helpers ─────────────────────────────────────────────────────────

func jsonResponse(w http.ResponseWriter, data interface{}) {
	w.Header().Set("Content-Type", "application/json")
	json.NewEncoder(w).Encode(data)
}

func jsonError(w http.ResponseWriter, msg string, code int) {
	w.Header().Set("Content-Type", "application/json")
	w.WriteHeader(code)
	json.NewEncoder(w).Encode(map[string]string{"error": msg})
}
